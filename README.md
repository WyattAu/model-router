# model-router

Cost-aware LLM routing core: per-model pricing tables, USD budget tracking,
and task-class routing with fallback chains.

This is the extracted, dependency-light routing core of
[clawdius](https://github.com/WyattAu/clawdius) — pure types and math, no HTTP
client, no provider SDK, no async runtime. It answers one question:

> given a task (and optionally a complexity requirement), which model should I
> call, what is the fallback order, and what did it cost?

```rust
use model_router::{CostTracker, ModelPricing, Router, RoutingRule, TaskClass};

let mut router = Router::new("claude-sonnet-4-20250514");
router.add_rule(
    RoutingRule::new(TaskClass::Fast, "claude-3-5-haiku-20241022")
        .with_fallback(RoutingRule::new(TaskClass::Fast, "gpt-4o-mini")),
);

// Route with a stable fallback chain.
let decision = router.route(TaskClass::Fast);
assert_eq!(decision.primary, "claude-3-5-haiku-20241022");
assert_eq!(
    decision.candidates().collect::<Vec<_>>(),
    ["claude-3-5-haiku-20241022", "gpt-4o-mini"]
);

// Estimate cost, then track it against a USD budget.
let pricing = router.pricing_for("claude-3-5-haiku-20241022").unwrap();
println!("estimate: ${:.4}", router.estimate_cost("claude-3-5-haiku-20241022", 10_000, 2_000));

let tracker = CostTracker::new(Some(5.0)); // $5.00 budget
tracker.record("claude-3-5-haiku-20241022", 10_000, 2_000, pricing).unwrap();
assert!(!tracker.is_over_budget());
println!("remaining: ${:.4}", tracker.remaining_usd().unwrap());
```

## What's inside

| Type | Purpose |
| --- | --- |
| [`ModelPricing`] | Input/output rates in USD per 1M tokens, context window, quality tier. |
| [`default_pricing_table`] | Built-in rates for popular Claude / GPT / Gemini / GLM / DeepSeek / OpenRouter models (May 2025). |
| [`CostTracker`] | Thread-safe spend accumulator; **checked-then-added** budget enforcement — an over-budget record is rejected and nothing is accumulated, so tracked spend can never exceed the limit. |
| [`TaskClass`] | Generic routing class: `Fast`, `Balanced`, `Power`, `Embedding`. |
| [`TaskComplexity`] | `Simple` → `Critical`; picks the cheapest quality tier that qualifies. |
| [`RoutingRule`] / [`Router`] | Task class → model, with a linked fallback chain and deterministic quality-per-dollar selection. |

## Mapping clawdius task types to `TaskClass`

clawdius used coding-workflow-shaped task types. The generic mapping:

| clawdius task type | `TaskClass` | typical model tier |
| --- | --- | --- |
| `Think` | `Fast` | cheap / fast |
| `Summarize` | `Fast` | cheap / fast |
| `Test` | `Fast` | cheap / fast |
| `Plan` | `Balanced` | mid tier |
| `Review` | `Balanced` | mid tier |
| `Chat` | `Balanced` | mid tier |
| `Build` | `Power` | strongest |
| *(embeddings)* | `Embedding` | embedding models |

`TaskClass::for_phase_name` implements this mapping for clawdius phase
vocabulary (`"think"`, `"plan"`, `"build"`, `"implement"`, `"code"`, `"test"`,
`"verify"`, `"review"`, `"reflect"`, `"summarize"`), with unknown names
falling back to `Balanced`.

## Features

- **`serde`** (off by default) — `Serialize`/`Deserialize` on the public data
  types.
- **`genai`** (off by default) — thin companion adapter mapping
  [`genai`](https://crates.io/crates/genai) `ModelIden` values to pricing
  entries (`model_router::genai_compat`). The core never depends on genai.

Other LLM clients work just as well: model keys are opaque strings, so any
client's model names can be used directly as pricing-table keys.

## Guarantees & precision notes

- `#![forbid(unsafe_code)]` and `#![deny(missing_docs)]`.
- Routing is deterministic: the pricing table is a `BTreeMap` and selection
  sorting is stable, so ties resolve to the alphabetically first model.
- `CostTracker` accumulates in fixed point at $0.0001 granularity,
  **truncating** — recorded deltas never exceed the true cost, totals err
  slightly low, and costs under $0.0001 truncate to zero. Budget enforcement
  runs in `f64` before any state is mutated.

## Testing

- 18 unit tests ported/derived from clawdius's `model_router.rs`.
- Property-based tests (`proptest`): cost accumulation monotonicity, budget
  never overshot when all spend flows through `record`, record/estimate
  consistency, and stable fallback ordering.
- Run: `cargo test` (add `--features genai` for the adapter tests).

## License

Apache-2.0 (matching clawdius).
