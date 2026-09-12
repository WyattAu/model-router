// Config-knob behavior matrix: every public config/builder knob must
// observably change routing, pricing, or budget behavior — default vs
// configured must differ. All tests are deterministic.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use model_router::{
    CostTracker, ModelPricing, PricingTable, Router, RoutingRule, TaskClass, TaskComplexity,
};

// ---------------------------------------------------------------------------
// Router::set_default_model — must change the decision for any class with
// no rule, and must NOT change decisions for classes with a rule.
// ---------------------------------------------------------------------------
#[test]
fn set_default_model_changes_unrouted_classes_only() {
    let mut router = Router::new("initial-default");
    assert_eq!(router.route(TaskClass::Balanced).primary, "initial-default");

    router.set_default_model("new-default");
    assert_eq!(router.route(TaskClass::Balanced).primary, "new-default");
    assert_eq!(router.default_model(), "new-default");
    // Default model yields no fallback chain.
    assert!(
        router.route(TaskClass::Balanced).fallbacks.is_empty(),
        "default-model decisions carry no fallbacks"
    );

    // A class with a rule is unaffected by the default-model knob.
    let mut router =
        Router::new("some-default").with_rule(RoutingRule::new(TaskClass::Fast, "fast-model"));
    router.set_default_model("other-default");
    assert_eq!(router.route(TaskClass::Fast).primary, "fast-model");
}

// ---------------------------------------------------------------------------
// Router::with_rule / add_rule — builder vs mutator must both change routing,
// and a later rule must replace the earlier one for the same class.
// ---------------------------------------------------------------------------
#[test]
fn with_rule_overrides_and_replaces_class_routing() {
    let mut router = Router::new("default-model");
    assert_eq!(router.route(TaskClass::Power).primary, "default-model");

    router = router.with_rule(RoutingRule::new(TaskClass::Power, "power-v1"));
    assert_eq!(router.route(TaskClass::Power).primary, "power-v1");

    // Replacement semantics: same class, new rule wins.
    router.add_rule(RoutingRule::new(TaskClass::Power, "power-v2"));
    assert_eq!(router.route(TaskClass::Power).primary, "power-v2");
    assert_eq!(router.rule_for(TaskClass::Power).unwrap().model, "power-v2");
}

// ---------------------------------------------------------------------------
// RoutingRule::with_fallback — the fallback knob shapes the decision's
// candidate order (primary first, then the chain in attach order).
// ---------------------------------------------------------------------------
#[test]
fn with_fallback_shapes_candidate_chain_order() {
    // Nested construction builds a two-hop chain.
    let rule = RoutingRule::new(TaskClass::Fast, "primary").with_fallback(
        RoutingRule::new(TaskClass::Fast, "fallback-1")
            .with_fallback(RoutingRule::new(TaskClass::Fast, "fallback-2")),
    );

    let router = Router::new("unused-default").with_rule(rule);
    let decision = router.route(TaskClass::Fast);
    assert_eq!(
        decision.candidates().collect::<Vec<_>>(),
        ["primary", "fallback-1", "fallback-2"]
    );

    // Calling with_fallback twice on the same rule REPLACES the fallback
    // (builder-style consume-and-mutate, not append): the last attachment wins.
    let replaced = RoutingRule::new(TaskClass::Fast, "primary")
        .with_fallback(RoutingRule::new(TaskClass::Fast, "first-attached"))
        .with_fallback(RoutingRule::new(TaskClass::Fast, "second-attached"));
    let router = Router::new("unused").with_rule(replaced);
    assert_eq!(
        router
            .route(TaskClass::Fast)
            .candidates()
            .collect::<Vec<_>>(),
        ["primary", "second-attached"]
    );

    // No fallback knob: candidates are exactly [primary].
    let bare = Router::new("unused").with_rule(RoutingRule::new(TaskClass::Fast, "only"));
    assert_eq!(
        bare.route(TaskClass::Fast).candidates().collect::<Vec<_>>(),
        ["only"]
    );
}

// ---------------------------------------------------------------------------
// Router::with_pricing / add_pricing — custom rates must flow into
// estimate_cost (default table vs overridden table must differ).
// ---------------------------------------------------------------------------
#[test]
fn with_pricing_changes_estimated_cost() {
    let default_estimate = Router::new("x").estimate_cost("glm-4.6", 1_000_000, 1_000_000);

    let custom = Router::new("x").with_pricing(
        "glm-4.6",
        ModelPricing::new(9.0, 9.0), // very different from built-in 0.5/0.5
    );
    let custom_estimate = custom.estimate_cost("glm-4.6", 1_000_000, 1_000_000);

    assert_eq!(
        default_estimate, 1.0,
        "built-in glm-4.6 is 0.5 + 0.5 per 1M"
    );
    assert_eq!(custom_estimate, 18.0);
    assert!(
        (custom_estimate - default_estimate).abs() > 1.0,
        "with_pricing must observably change the estimate"
    );

    // add_pricing for an unknown model replaces the placeholder estimate.
    let mut router = Router::new("x");
    let placeholder = router.estimate_cost("brand-new-model", 1_000_000, 0);
    assert_eq!(placeholder, 1.0, "placeholder pricing is $1/1M input");
    router.add_pricing("brand-new-model", ModelPricing::new(4.0, 0.0));
    assert_eq!(router.estimate_cost("brand-new-model", 1_000_000, 0), 4.0);
}

// ---------------------------------------------------------------------------
// Router::merge_pricing — freshness-aware merge must replace stamped-newer
// entries (observable through estimate_cost) while preserving equal/older.
// ---------------------------------------------------------------------------
#[test]
fn merge_pricing_applies_fresher_entries_only() {
    let mut router = Router::new("x");
    let before = router.estimate_cost("glm-4.6", 1_000_000, 0);

    let mut fresh = PricingTable::new();
    fresh.insert(
        "glm-4.6",
        ModelPricing::new(7.5, 0.5).with_updated_at_unix(1_757_289_600),
    );
    router.merge_pricing(&fresh);
    let after_fresh = router.estimate_cost("glm-4.6", 1_000_000, 0);
    assert_eq!(after_fresh, 7.5);
    assert!(
        (after_fresh - before).abs() > 1.0,
        "merging a stamped-newer entry must change the estimate"
    );

    // A stale (older-stamped) merge must not win over the 0.1.1-stamped one.
    let mut stale = PricingTable::new();
    stale.insert(
        "glm-4.6",
        ModelPricing::new(99.0, 99.0).with_updated_at_unix(1),
    );
    router.merge_pricing(&stale);
    assert_eq!(
        router.estimate_cost("glm-4.6", 1_000_000, 0),
        7.5,
        "older-stamped entry must be ignored"
    );
}

// ---------------------------------------------------------------------------
// ModelPricing::with_quality_tier — flows into select_by_complexity:
// a tier below the complexity's minimum excludes the model; raising the
// tier includes it, and cheaper-meets-tier wins on efficiency.
// ---------------------------------------------------------------------------
#[test]
fn with_quality_tier_gates_select_by_complexity() {
    // NOTE: Router::new seeds the built-in pricing table, so expectations are
    // stated relative to it. No built-in tier-5 model is cheap, so a very
    // cheap custom entry dominates Critical cleanly.

    // Tier below the complexity minimum: excluded even though it is the
    // cheapest model in the table by far.
    let mut router = Router::new("x");
    router.add_pricing("m", ModelPricing::new(0.0001, 0.0001).with_quality_tier(4));
    let without_m = router
        .select_by_complexity(TaskComplexity::Critical)
        .unwrap();
    assert_ne!(
        without_m, "m",
        "tier 4 is below Critical's minimum of 5 — built-in tier-5 must win"
    );

    // Raising the SAME model's tier to 5 makes it eligible, and its price
    // wins on efficiency over every built-in tier-5 model.
    let mut router = Router::new("x");
    router.add_pricing("m", ModelPricing::new(0.0001, 0.0001).with_quality_tier(5));
    assert_eq!(
        router.select_by_complexity(TaskComplexity::Critical),
        Some("m")
    );
    assert_eq!(
        router.select_by_complexity(TaskComplexity::Medium),
        Some("m")
    );

    // Between two eligible custom models, the cheaper (higher-efficiency) wins.
    let mut router = Router::new("x");
    router.add_pricing(
        "cheap-t5",
        ModelPricing::new(0.0001, 0.0001).with_quality_tier(5),
    );
    router.add_pricing(
        "pricey-t5",
        ModelPricing::new(10.0, 10.0).with_quality_tier(5),
    );
    assert_eq!(
        router.select_by_complexity(TaskComplexity::Critical),
        Some("cheap-t5")
    );
}

// ---------------------------------------------------------------------------
// ModelPricing::with_context_window / with_max_output_tokens — data knobs;
// they must round-trip through the builders into the public fields (they are
// metadata for callers' request shaping, not routing inputs).
// ---------------------------------------------------------------------------
#[test]
fn context_and_output_limits_round_trip() {
    let pricing = ModelPricing::new(1.0, 2.0)
        .with_context_window(1_000_000)
        .with_max_output_tokens(32_768);
    assert_eq!(pricing.context_window, 1_000_000);
    assert_eq!(pricing.max_output_tokens, 32_768);

    let defaults = ModelPricing::default();
    assert_ne!(
        pricing.context_window, defaults.context_window,
        "configured context window must differ from the 128k default"
    );
}

// ---------------------------------------------------------------------------
// CostTracker budget knob — `new(Some(limit))` must enforce: a record past
// the limit is rejected and accumulates nothing; `new(None)` accepts.
// ---------------------------------------------------------------------------
#[test]
fn budget_limit_enforces_and_reports() {
    let expensive = ModelPricing::new(1.0, 0.0); // $1 per 1M input
    let limited = CostTracker::new(Some(0.001)); // $0.001 budget

    limited.record("m", 1_000, 0, &expensive).unwrap(); // $0.001 — exactly at budget
    let rejected = limited.record("m", 1_000, 0, &expensive).unwrap_err();
    assert!(
        matches!(rejected, model_router::RouterError::BudgetExceeded { .. }),
        "second record must be rejected past the $0.001 limit"
    );
    assert_eq!(
        limited.total_usd(),
        0.001,
        "rejected record must not accumulate"
    );
    assert!(limited.is_over_budget());
    assert_eq!(limited.remaining_usd(), Some(0.0));
    assert_eq!(limited.report().budget_limit_usd, Some(0.001));

    // Unlimited tracker: same records accepted.
    let unlimited = CostTracker::new(None);
    unlimited.record("m", 10_000_000, 0, &expensive).unwrap();
    assert!(!unlimited.is_over_budget());
    assert_eq!(unlimited.remaining_usd(), None);
}

// ---------------------------------------------------------------------------
// Router::with_default_rules — provider knob picks the documented cheap
// model for Fast while Power/Balanced ride the primary; unknown providers
// fall back to the primary everywhere.
// ---------------------------------------------------------------------------
#[test]
fn default_rules_map_provider_to_cheap_model() {
    let router = Router::with_default_rules("anthropic", "claude-sonnet-4-20250514");
    assert_eq!(
        router.route(TaskClass::Fast).primary,
        "claude-3-5-haiku-20241022"
    );
    assert_eq!(
        router.route(TaskClass::Power).primary,
        "claude-sonnet-4-20250514"
    );
    assert_eq!(
        router.route(TaskClass::Balanced).primary,
        "claude-sonnet-4-20250514"
    );

    let unknown = Router::with_default_rules("acme", "acme-top");
    assert_eq!(unknown.route(TaskClass::Fast).primary, "acme-top");

    // Embedding is deliberately unconfigured: falls to the default model.
    assert_eq!(
        router.route(TaskClass::Embedding).primary,
        "claude-sonnet-4-20250514"
    );
}

// ---------------------------------------------------------------------------
// Router::pricing_table / pricing_for expose merged state — the knobs above
// are observable through these accessors too (guards against a knob that
// mutates a shadow copy).
// ---------------------------------------------------------------------------
#[test]
fn pricing_knobs_are_visible_through_accessors() {
    let router = Router::new("x").with_pricing("m", ModelPricing::new(2.0, 3.0));
    assert_eq!(router.pricing_for("m").unwrap().input_per_1m, 2.0);
    let table: BTreeMap<_, _> = router.pricing_table().clone();
    assert_eq!(table["m"].output_per_1m, 3.0);
}
