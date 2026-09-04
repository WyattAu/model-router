//! Unit tests ported from clawdius `model_router.rs` (pure seam only), plus
//! coverage for the seams that file's extraction created (fallback chains,
//! unknown-model estimation, report snapshots).

use model_router::{
    default_pricing_table, CostReport, CostTracker, ModelPricing, Router, RouterError, RoutingRule,
    TaskClass, TaskComplexity,
};

fn sonnet_pricing() -> ModelPricing {
    ModelPricing::new(3.0, 15.0)
        .with_context_window(200_000)
        .with_max_output_tokens(16_384)
        .with_quality_tier(5)
}

// --- ported: test_task_type_from_phase -------------------------------------

#[test]
fn test_task_class_for_phase_name() {
    // clawdius vocabulary maps onto the generic classes.
    assert_eq!(TaskClass::for_phase_name("think"), TaskClass::Fast);
    assert_eq!(TaskClass::for_phase_name("Think"), TaskClass::Fast);
    assert_eq!(TaskClass::for_phase_name("test"), TaskClass::Fast);
    assert_eq!(TaskClass::for_phase_name("summarize"), TaskClass::Fast);
    assert_eq!(TaskClass::for_phase_name("build"), TaskClass::Power);
    assert_eq!(TaskClass::for_phase_name("implement"), TaskClass::Power);
    assert_eq!(TaskClass::for_phase_name("code"), TaskClass::Power);
    assert_eq!(TaskClass::for_phase_name("plan"), TaskClass::Balanced);
    assert_eq!(TaskClass::for_phase_name("review"), TaskClass::Balanced);
    assert_eq!(TaskClass::for_phase_name("chat"), TaskClass::Balanced);
    assert_eq!(
        TaskClass::for_phase_name("embeddings"),
        TaskClass::Embedding
    );
    // Unknown names fall back to Balanced (clawdius fell back to Chat).
    assert_eq!(TaskClass::for_phase_name("unknown"), TaskClass::Balanced);
}

// --- ported: test_model_pricing_cost ----------------------------------------

#[test]
fn test_model_pricing_cost() {
    let claude_sonnet = sonnet_pricing();

    // 1k input, 500 output tokens
    let cost = claude_sonnet.cost(1000, 500);
    assert!((cost - 0.0105).abs() < 0.0001); // $0.0105

    // 100k input, 10k output tokens
    let cost = claude_sonnet.cost(100_000, 10_000);
    assert!((cost - 0.45).abs() < 0.01); // ~$0.45
}

// --- ported: test_free_model_has_zero_cost ----------------------------------

#[test]
fn test_free_model_has_zero_cost() {
    let free = ModelPricing::new(0.0, 0.0);
    assert_eq!(free.cost(1_000_000, 1_000_000), 0.0);
    assert_eq!(free.efficiency(), f64::MAX);
}

// --- ported: test_cost_tracker_basic (now sync) -----------------------------

#[test]
fn test_cost_tracker_basic() {
    let tracker = CostTracker::new(None);
    let pricing = sonnet_pricing();

    tracker
        .record("claude-sonnet-4", 1000, 500, &pricing)
        .unwrap();
    tracker
        .record("claude-sonnet-4", 2000, 1000, &pricing)
        .unwrap();

    assert!(tracker.total_usd() > 0.0);
    assert_eq!(tracker.total_input_tokens(), 3000);
    assert_eq!(tracker.total_output_tokens(), 1500);

    let breakdown = tracker.per_model_breakdown();
    assert!(breakdown.contains_key("claude-sonnet-4"));
    assert_eq!(breakdown["claude-sonnet-4"].request_count, 2);
}

// --- ported: test_cost_tracker_budget_enforcement ---------------------------

#[test]
fn test_cost_tracker_budget_enforcement() {
    let tracker = CostTracker::new(Some(0.01)); // $0.01 budget

    let expensive = ModelPricing::new(100.0, 500.0);

    // First request should succeed ($0.001 + $0.0025 = $0.0035)
    tracker
        .record("expensive-model", 10, 5, &expensive)
        .unwrap();
    // Second request exceeds budget — should be rejected
    let result = tracker.record("expensive-model", 100_000, 10_000, &expensive);
    assert!(matches!(result, Err(RouterError::BudgetExceeded { .. })));
    // Budget was not exceeded because the over-budget request was rejected
    assert!(!tracker.is_over_budget());
    // But total is close to budget
    assert!(tracker.total_usd() < 0.01);
    // Remaining budget reflects the limit minus spend
    let remaining = tracker.remaining_usd().unwrap();
    assert!((remaining - (0.01 - tracker.total_usd())).abs() < 1e-9);
}

// --- ported: test_default_pricing_table --------------------------------------

#[test]
fn test_default_pricing_table() {
    let table = default_pricing_table();
    assert!(table.contains_key("claude-sonnet-4-20250514"));
    assert!(table.contains_key("gpt-4o"));
    assert!(table.contains_key("glm-4.6"));
    assert!(table.contains_key("gemini-2.0-flash"));

    // Free models
    assert_eq!(table["google/gemma-3-4b-it:free"].input_per_1m, 0.0);
}

// --- ported: test_default_rules -----------------------------------------------

#[test]
fn test_default_rules() {
    let rules = Router::default_rules("anthropic", "claude-sonnet-4-20250514");
    assert_eq!(rules.len(), 3);

    // Fast should use the cheap model
    let fast_rule = rules.iter().find(|r| r.task == TaskClass::Fast).unwrap();
    assert_eq!(fast_rule.model, "claude-3-5-haiku-20241022");

    // Power should use the primary model
    let power_rule = rules.iter().find(|r| r.task == TaskClass::Power).unwrap();
    assert_eq!(power_rule.model, "claude-sonnet-4-20250514");

    // Unknown providers fall back to the primary model for everything.
    let rules = Router::default_rules("mystery-provider", "my-model");
    assert!(rules.iter().all(|r| r.model == "my-model"));
}

// --- ported: test_estimate_cost ------------------------------------------------

#[test]
fn test_estimate_cost() {
    let router = Router::new("glm-4.6");

    // GLM-4.6: $0.50/1M input, $0.50/1M output
    let cost = router.estimate_cost("glm-4.6", 100_000, 10_000);
    assert!((cost - 0.055).abs() < 0.001); // ~$0.055

    // Unknown models estimate with placeholder pricing ($1/$3 per 1M).
    let cost = router.estimate_cost("not-in-table", 1_000_000, 1_000_000);
    assert!((cost - 4.0).abs() < 1e-9);
}

// --- ported: test_model_pricing_efficiency ---------------------------------------

#[test]
fn test_model_pricing_efficiency() {
    let cheap = ModelPricing::new(0.15, 0.6).with_quality_tier(3);
    let expensive = ModelPricing::new(15.0, 75.0).with_quality_tier(5);

    // Cheap model should have higher efficiency (quality per dollar)
    assert!(cheap.efficiency() > expensive.efficiency());
}

// --- ported: test_task_complexity_ordering ----------------------------------------

#[test]
fn test_task_complexity_ordering() {
    assert!(TaskComplexity::Simple < TaskComplexity::Medium);
    assert!(TaskComplexity::Medium < TaskComplexity::Complex);
    assert!(TaskComplexity::Complex < TaskComplexity::Critical);

    assert_eq!(TaskComplexity::Simple.min_quality_tier(), 2);
    assert_eq!(TaskComplexity::Critical.min_quality_tier(), 5);
}

// --- ported: test_select_model_for_complexity ---------------------------------------

#[test]
fn test_select_model_for_complexity() {
    let router = Router::new("gpt-4o");

    // Both ends of the spectrum select something from the built-in table.
    let simple = router.select_by_complexity(TaskComplexity::Simple);
    assert!(simple.is_some());

    let critical = router.select_by_complexity(TaskComplexity::Critical);
    assert!(critical.is_some());

    // The critical pick must meet tier 5 and be from the priced table.
    let critical = critical.unwrap();
    let pricing = router.pricing_for(critical).unwrap();
    assert!(pricing.quality_tier >= 5);
}

// --- ported: test_cost_report ---------------------------------------------------------

#[test]
fn test_cost_report() {
    let tracker = CostTracker::new(Some(10.0));
    let pricing = ModelPricing::new(1.0, 3.0);

    tracker.record("test-model", 1000, 500, &pricing).unwrap();
    let report: CostReport = tracker.report();
    assert_eq!(report.per_model.len(), 1);
    assert!(!report.is_over_budget);
    assert!(report.total_cost_usd > 0.0);
    assert_eq!(report.budget_limit_usd, Some(10.0));
}

// --- new: fallback chain resolution -----------------------------------------------------

#[test]
fn test_fallback_chain_order_is_stable() {
    let mut router = Router::new("fallback-final");
    router.add_rule(
        RoutingRule::new(TaskClass::Power, "claude-sonnet-4-20250514").with_fallback(
            RoutingRule::new(TaskClass::Power, "gpt-4o")
                .with_fallback(RoutingRule::new(TaskClass::Power, "glm-4.6")),
        ),
    );

    let decision = router.route(TaskClass::Power);
    assert_eq!(decision.primary, "claude-sonnet-4-20250514");
    let candidates: Vec<_> = decision.candidates().collect();
    assert_eq!(
        candidates,
        ["claude-sonnet-4-20250514", "gpt-4o", "glm-4.6"]
    );

    // Deterministic across calls.
    let again = router.route(TaskClass::Power);
    assert_eq!(decision, again);
}

#[test]
fn test_route_without_rule_uses_default_model() {
    let router = Router::new("claude-sonnet-4-20250514");
    let decision = router.route(TaskClass::Embedding);
    assert_eq!(decision.primary, "claude-sonnet-4-20250514");
    assert!(decision.fallbacks.is_empty());
    assert_eq!(decision.candidates().count(), 1);
}

#[test]
fn test_with_default_rules_router() {
    let router = Router::with_default_rules("openai", "gpt-4o");
    assert_eq!(router.route(TaskClass::Fast).primary, "gpt-4o-mini");
    assert_eq!(router.route(TaskClass::Power).primary, "gpt-4o");
}

#[test]
fn test_clone_shares_tracker_state() {
    let tracker = CostTracker::new(None);
    let clone = tracker.clone();
    let pricing = sonnet_pricing();
    tracker.record("m", 1000, 0, &pricing).unwrap();
    assert!(clone.total_usd() > 0.0);
    assert_eq!(clone.total_input_tokens(), 1000);
}

#[test]
fn test_display_and_error_message() {
    let tracker = CostTracker::new(Some(0.001));
    let pricing = sonnet_pricing();
    let err = tracker.record("m", 100_000, 10_000, &pricing).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("budget exceeded"), "unexpected: {msg}");

    assert_eq!(TaskClass::Fast.to_string(), "fast");
    assert_eq!(TaskClass::Embedding.to_string(), "embedding");
}

#[test]
fn test_serde_round_trip() {
    // Compiled with the `serde` feature in CI; guard for the default build.
    #[cfg(feature = "serde")]
    {
        let rule = RoutingRule::new(TaskClass::Fast, "gpt-4o-mini")
            .with_fallback(RoutingRule::new(TaskClass::Fast, "glm-4.6"));
        let json = serde_json::to_string(&rule).unwrap();
        let back: RoutingRule = serde_json::from_str(&json).unwrap();
        assert_eq!(rule, back);

        let pricing = sonnet_pricing();
        let json = serde_json::to_string(&pricing).unwrap();
        let back: ModelPricing = serde_json::from_str(&json).unwrap();
        assert_eq!(pricing, back);
    }
}
