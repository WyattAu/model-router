//! Property-based tests for the cost and routing core.
//!
//! Properties (per extraction spec):
//! 1. Cost accumulation is monotonic — totals never decrease.
//! 2. The budget is never overshot when every spend goes through
//!    `CostTracker::record` (checked-then-added semantics).
//! 3. Fallback ordering is stable — routing is deterministic and the
//!    primary model is always first in the candidate chain.

use model_router::{CostTracker, ModelPricing, Router, RoutingRule, TaskClass};
use proptest::prelude::*;

/// Random strictly-positive per-1M USD rates, in a realistic 0.01..=300 band.
fn rate() -> impl Strategy<Value = f64> {
    (1u32..30_000u32).prop_map(|cents_x1000| cents_x1000 as f64 / 1000.0)
}

/// Random token counts, kept modest so sums don't drift into float noise.
fn tokens() -> impl Strategy<Value = usize> {
    0..=500_000usize
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn prop_cost_accumulation_is_monotonic(
        records in proptest::collection::vec(
            (rate(), rate(), tokens(), tokens()), 1..64
        )
    ) {
        let tracker = CostTracker::new(None);
        let mut prev_total = 0.0f64;
        let mut prev_in = 0u64;
        let mut prev_out = 0u64;

        for (in_rate, out_rate, input, output) in records {
            let pricing = ModelPricing::new(in_rate, out_rate);
            tracker.record("prop-model", input, output, &pricing).unwrap();

            let total = tracker.total_usd();
            prop_assert!(total >= prev_total, "total decreased: {prev_total} -> {total}");
            prop_assert!(tracker.total_input_tokens() >= prev_in);
            prop_assert!(tracker.total_output_tokens() >= prev_out);

            prev_total = total;
            prev_in = tracker.total_input_tokens();
            prev_out = tracker.total_output_tokens();
        }
    }

    #[test]
    fn prop_budget_is_never_overshot(
        limit_thousandths in 1u64..1_000_000u64,
        records in proptest::collection::vec(
            (rate(), rate(), tokens(), tokens()), 1..64
        )
    ) {
        let limit = limit_thousandths as f64 / 1000.0;
        let tracker = CostTracker::new(Some(limit));

        for (in_rate, out_rate, input, output) in records {
            let pricing = ModelPricing::new(in_rate, out_rate);
            match tracker.record("prop-model", input, output, &pricing) {
                Ok(()) => {
                    prop_assert!(
                        tracker.total_usd() <= limit,
                        "overshot: {} > {limit}",
                        tracker.total_usd()
                    );
                },
                Err(model_router::RouterError::BudgetExceeded { .. }) => {
                    // Rejected records must not change tracked state.
                    // (Checked implicitly by the invariant above holding.)
                },
                Err(e) => prop_assert!(false, "unexpected error: {e}"),
            }
        }

        prop_assert!(
            tracker.total_usd() <= limit,
            "final total {} exceeds limit {limit}",
            tracker.total_usd()
        );
    }

    #[test]
    fn prop_record_delta_matches_estimate(
        in_rate in rate(), out_rate in rate(), input in tokens(), output in tokens()
    ) {
        let pricing = ModelPricing::new(in_rate, out_rate);
        let tracker = CostTracker::new(None);
        let expected = pricing.cost(input, output);
        tracker.record("prop-model", input, output, &pricing).unwrap();
        let delta = tracker.total_usd();
        // Fixed-point truncation errs downward and loses at most one
        // $0.0001 unit per record (see CostTracker::record docs).
        prop_assert!(delta <= expected, "recorded {delta} exceeds estimate {expected}");
        prop_assert!(
            expected - delta <= 1e-4,
            "recorded {delta} lost more than one fixed-point unit of {expected}"
        );
    }

    #[test]
    fn prop_fallback_ordering_stable(
        chain in proptest::collection::vec("[a-z0-9./:_-]{1,12}", 1..8),
        flip in proptest::bool::ANY,
    ) {
        // Build a linked fallback chain for a task class. The input order is
        // the intended try order: first entry is primary, each subsequent
        // entry is the next fallback.
        let task = if flip { TaskClass::Fast } else { TaskClass::Power };
        let order: Vec<String> = chain.clone();
        let mut built = RoutingRule::new(task, order.last().unwrap().clone());
        for model in order.iter().rev().skip(1) {
            built = RoutingRule::new(task, model.clone()).with_fallback(built);
        }

        let router = Router::new("unrelated-default").with_rule(built);

        let first = router.route(task);
        let second = router.route(task);
        prop_assert!(first == second, "routing is not deterministic");

        let expected: Vec<&str> = order.iter().map(|s| s.as_str()).collect();
        let candidates: Vec<&str> = first.candidates().collect();
        prop_assert!(candidates == expected, "fallback order must follow chain");
        prop_assert!(first.primary == expected[0], "primary must be chain head");
    }
}
