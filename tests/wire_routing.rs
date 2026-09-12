// Wire tests drive the router against a live mock provider; unwrap/expect
// is the test signal here.
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Routing wire tests against a mock OpenAI-compatible provider
//! (wiremock on loopback).
//!
//! `model-router` is deliberately dependency-light: it decides *which*
//! model to call and *what it cost*, but has no HTTP client of its own.
//! The optional `genai` companion cannot target a local server (genai
//! hardcodes provider endpoints), so this suite tests at the router seam:
//! [`Router`] decisions drive a thin reqwest provider client against real
//! HTTP servers speaking the OpenAI chat-completions shape. That pins:
//!
//! - request shape (model field = the routed candidate, messages array);
//! - failover: primary 500 → fallback 200, walking `candidates()` in order;
//! - cost accounting: usage from the wire → `CostTracker::record` → budget
//!   enforcement;
//! - cost-aware selection: `select_by_complexity` picks the cheapest model
//!   meeting the quality floor.

use std::sync::Arc;

use model_router::{
    CostTracker, ModelPricing, PricingTable, Router, RouterError, RoutingRule, TaskClass,
    TaskComplexity,
};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Usage numbers an OpenAI-compatible provider reports.
#[derive(Debug, Clone, Copy)]
struct Usage {
    prompt_tokens: usize,
    completion_tokens: usize,
}

/// Thin provider client: POST {base}/v1/chat/completions with the OpenAI
/// request shape, returning the usage block from the response.
async fn chat_completion(
    http: &reqwest::Client,
    base: &str,
    model: &str,
    prompt: &str,
) -> Result<Usage, String> {
    let url = format!("{base}/v1/chat/completions");
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 128,
    });
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("provider {base} returned {status}"));
    }
    let payload: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let usage = &payload["usage"];
    Ok(Usage {
        prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0) as usize,
        completion_tokens: usage["completion_tokens"].as_u64().unwrap_or(0) as usize,
    })
}

fn ok_completion(model: &str, prompt_tokens: u64, completion_tokens: u64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "chatcmpl-test-1",
        "object": "chat.completion",
        "created": 1_760_000_000u64,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "done"},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens + completion_tokens
        }
    }))
}

// ---------------------------------------------------------------------------
// Primary route + cost accounting over the wire
// ---------------------------------------------------------------------------

#[tokio::test]
async fn chat_completion_routes_to_primary_and_records_real_usage_cost() {
    let server = MockServer::start().await;
    // Pin the exact request shape the router's decision produces.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({
            "model": "haiku-test",
            "messages": [{"role": "user", "content": "summarize this"}]
        })))
        .respond_with(ok_completion("haiku-test", 1_000, 200))
        .expect(1)
        .mount(&server)
        .await;

    let mut router = Router::new("default-model");
    router.add_rule(RoutingRule::new(TaskClass::Fast, "haiku-test"));
    router.add_pricing("haiku-test", ModelPricing::new(0.80, 4.00)); // USD / 1M

    let decision = router.route(TaskClass::Fast);
    assert_eq!(decision.primary, "haiku-test");

    let http = reqwest::Client::new();
    let usage = chat_completion(&http, &server.uri(), decision.primary, "summarize this")
        .await
        .unwrap();
    assert_eq!(usage.prompt_tokens, 1_000);
    assert_eq!(usage.completion_tokens, 200);

    // Cost from the wire usage: 1000/1M * 0.80 + 200/1M * 4.00 = 0.0016.
    let cost = router.estimate_cost(
        decision.primary,
        usage.prompt_tokens,
        usage.completion_tokens,
    );
    assert!((cost - 0.0016).abs() < 1e-9, "unexpected cost: {cost}");

    let tracker = CostTracker::new(None);
    tracker
        .record(
            decision.primary,
            usage.prompt_tokens,
            usage.completion_tokens,
            router.pricing_for(decision.primary).unwrap(),
        )
        .unwrap();
    assert!((tracker.total_usd() - 0.0016).abs() < 1e-6);
    let report = tracker.report();
    assert_eq!(report.per_model.len(), 1);
    assert!(tracker.total_input_tokens() == 1_000 && tracker.total_output_tokens() == 200);

    server.verify().await;
}

// ---------------------------------------------------------------------------
// Failover: primary 500 → fallback 200
// ---------------------------------------------------------------------------

#[tokio::test]
async fn failover_walks_the_candidate_chain_on_provider_failure() {
    let primary = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string("upstream exploded"))
        .expect(1) // exactly one attempt against the dead primary
        .mount(&primary)
        .await;

    let secondary = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "model": "gpt-mini-test" })))
        .respond_with(ok_completion("gpt-mini-test", 500, 100))
        .expect(1)
        .mount(&secondary)
        .await;

    let mut router = Router::new("default-model");
    router.add_rule(
        RoutingRule::new(TaskClass::Balanced, "sonnet-test")
            .with_fallback(RoutingRule::new(TaskClass::Balanced, "gpt-mini-test")),
    );
    router.add_pricing("sonnet-test", ModelPricing::new(3.0, 15.0));
    router.add_pricing("gpt-mini-test", ModelPricing::new(0.15, 0.60));

    let decision = router.route(TaskClass::Balanced);
    let candidates: Vec<&str> = decision.candidates().collect();
    assert_eq!(candidates, vec!["sonnet-test", "gpt-mini-test"]);

    // The caller-side failover loop implied by `candidates()`: try each in
    // order until one succeeds.
    let http = reqwest::Client::new();
    let bases = [primary.uri(), secondary.uri()];
    let mut used = None;
    for (i, model) in candidates.iter().enumerate() {
        match chat_completion(&http, &bases[i], model, "plan this").await {
            Ok(usage) => {
                used = Some((model.to_string(), usage));
                break;
            }
            Err(_) => continue,
        }
    }
    let (model, usage) = used.expect("fallback must have succeeded");
    assert_eq!(model, "gpt-mini-test", "traffic must land on the fallback");
    assert_eq!(usage.prompt_tokens, 500);

    // Cost is accounted under the model that actually served, at fallback
    // rates: 500/1M * 0.15 + 100/1M * 0.60 = 0.000135.
    let cost = router.estimate_cost(&model, usage.prompt_tokens, usage.completion_tokens);
    assert!(
        (cost - 0.000135).abs() < 1e-9,
        "fallback must be billed at fallback rates: {cost}"
    );

    primary.verify().await;
    secondary.verify().await;
}

#[tokio::test]
async fn all_candidates_failing_is_reported_not_silent() {
    let primary = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&primary)
        .await;
    let secondary = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&secondary)
        .await;

    let mut router = Router::new("default-model");
    router.add_rule(
        RoutingRule::new(TaskClass::Power, "opus-test")
            .with_fallback(RoutingRule::new(TaskClass::Power, "gpt-big-test")),
    );

    let decision = router.route(TaskClass::Power);
    let http = reqwest::Client::new();
    let bases = [primary.uri(), secondary.uri()];
    let mut last_err = String::new();
    for (i, model) in decision.candidates().enumerate() {
        if let Ok(usage) = chat_completion(&http, &bases[i], model, "build this").await {
            let _ = usage;
            panic!("no candidate should have succeeded");
        } else {
            last_err = "failed".to_string();
        }
    }
    assert_eq!(
        last_err, "failed",
        "every candidate failed — caller must surface this"
    );
}

// ---------------------------------------------------------------------------
// Timeout handling
// ---------------------------------------------------------------------------

#[tokio::test]
async fn slow_provider_is_abandoned_for_the_fallback() {
    let slow = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({}))
                .set_delay(std::time::Duration::from_secs(5)),
        )
        .mount(&slow)
        .await;

    let fast = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok_completion("fast-model", 10, 5))
        .expect(1)
        .mount(&fast)
        .await;

    let mut router = Router::new("default-model");
    router.add_rule(
        RoutingRule::new(TaskClass::Fast, "slow-model")
            .with_fallback(RoutingRule::new(TaskClass::Fast, "fast-model")),
    );

    // Client-side timeout of 300 ms: the slow primary is abandoned and the
    // fallback answers within the same logical call.
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(300))
        .build()
        .unwrap();
    let decision = router.route(TaskClass::Fast);
    let bases = [slow.uri(), fast.uri()];
    let mut served_by = None;
    for (i, model) in decision.candidates().enumerate() {
        match chat_completion(&http, &bases[i], model, "quick question").await {
            Ok(_) => {
                served_by = Some(model.to_string());
                break;
            }
            Err(_) => continue,
        }
    }
    assert_eq!(
        served_by.as_deref(),
        Some("fast-model"),
        "the timed-out primary must be skipped"
    );
    fast.verify().await;
}

// ---------------------------------------------------------------------------
// Cost-aware selection + budget enforcement
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cost_aware_selection_meets_quality_floor_at_lowest_cost() {
    let mut router = Router::new("default-model");
    // Efficiency = quality_tier / avg_cost, sorted descending. Values below
    // are extreme enough to beat every built-in default entry, keeping the
    // selection deterministic.
    router.add_pricing(
        "cheap-weak",
        ModelPricing::new(0.05, 0.20).with_quality_tier(2),
    );
    router.add_pricing(
        "cheap-strong",
        ModelPricing::new(0.01, 0.02).with_quality_tier(3),
    );
    router.add_pricing(
        "pricey-strong",
        ModelPricing::new(3.00, 15.00).with_quality_tier(3),
    );
    router.add_pricing(
        "critical-tier",
        ModelPricing::new(1.00, 2.00).with_quality_tier(5),
    );

    // Medium requires tier >= 3: the cheapest tier-3 model wins over the
    // pricier tier-3 models and the tier-2 cheapie is excluded.
    let medium = router.select_by_complexity(TaskComplexity::Medium).unwrap();
    assert_eq!(medium, "cheap-strong");

    // Critical requires tier >= 5: only the tier-5 model qualifies.
    let critical = router
        .select_by_complexity(TaskComplexity::Critical)
        .unwrap();
    assert_eq!(critical, "critical-tier");

    // Wire check: routing a build task through the selected model really
    // reaches a server speaking to that model name.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({ "model": "cheap-strong" })))
        .respond_with(ok_completion("cheap-strong", 64, 32))
        .expect(1)
        .mount(&server)
        .await;
    let http = reqwest::Client::new();
    let usage = chat_completion(&http, &server.uri(), medium, "fix this bug")
        .await
        .unwrap();
    assert_eq!(usage.prompt_tokens, 64);
    server.verify().await;
}

#[tokio::test]
async fn budget_cap_rejects_records_that_would_exceed_it() {
    let tracker = Arc::new(CostTracker::new(Some(0.01)));

    // $0.008 fits.
    tracker
        .record("m", 1_000, 1_000, &ModelPricing::new(4.0, 4.0))
        .unwrap();
    assert!((tracker.total_usd() - 0.008).abs() < 1e-9);

    // Another $0.008 would push past the $0.01 cap: rejected before any
    // state mutation.
    let err = tracker
        .record("m", 1_000, 1_000, &ModelPricing::new(4.0, 4.0))
        .unwrap_err();
    assert!(
        matches!(err, RouterError::BudgetExceeded { .. }),
        "over-budget records must be rejected: {err:?}"
    );
    assert!(
        (tracker.total_usd() - 0.008).abs() < 1e-9,
        "spend must not move on rejection"
    );
    assert!(!tracker.is_over_budget());
    assert_eq!(tracker.remaining_usd(), Some(0.002));
}

#[tokio::test]
async fn pricing_refresh_via_json_updates_cost_accounting() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok_completion("haiku-test", 1_000_000, 0))
        .expect(1)
        .mount(&server)
        .await;

    // The refresh workflow stamps entries with refresh time; merge is
    // newer-wins on `updated_at_unix`, so the new snapshot must carry a
    // newer stamp than the table it replaces.
    let old_table: PricingTable =
        PricingTable::from_json(r#"{"haiku-test": {"input_per_1m": 1.0, "output_per_1m": 1.0, "context_window": 128000, "max_output_tokens": 4096, "quality_tier": 3, "updated_at_unix": 1000}}"#).unwrap();
    let new_table: PricingTable =
        PricingTable::from_json(r#"{"haiku-test": {"input_per_1m": 2.0, "output_per_1m": 2.0, "context_window": 200000, "max_output_tokens": 8192, "quality_tier": 3, "updated_at_unix": 2000}}"#).unwrap();

    let mut router = Router::new("default-model");
    router.merge_pricing(&old_table);
    let http = reqwest::Client::new();

    // Pre-refresh price: 1M input tokens at $1/1M = $1.
    let usage = chat_completion(&http, &server.uri(), "haiku-test", "x")
        .await
        .unwrap();
    let cost_before =
        router.estimate_cost("haiku-test", usage.prompt_tokens, usage.completion_tokens);
    assert!((cost_before - 1.0).abs() < 1e-9);

    // Refresh the table (the price-feed workflow) and re-price.
    router.merge_pricing(&new_table);
    let cost_after =
        router.estimate_cost("haiku-test", usage.prompt_tokens, usage.completion_tokens);
    assert!(
        (cost_after - 2.0).abs() < 1e-9,
        "refreshed rates must apply: {cost_after}"
    );
    assert!(cost_after > cost_before);

    server.verify().await;
}
