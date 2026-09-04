//! Cost-aware LLM routing: pricing tables, USD budgets, task-class routing.
//!
//! `model-router` is the extracted, dependency-light core of the router built
//! for [clawdius](https://github.com/WyattAu/clawdius). It contains only pure
//! types and math:
//!
//! - [`ModelPricing`] — per-model input/output rates in USD per 1M tokens.
//! - [`CostTracker`] — thread-safe accumulation of spend against an optional
//!   USD budget; rejects any single record that would push spend over budget.
//! - [`TaskClass`] — a small generic routing class (`Fast` / `Balanced` /
//!   `Power` / `Embedding`).
//! - [`RoutingRule`] + [`Router`] — task class to model selection with a
//!   linked fallback chain, plus quality-tier-aware selection by
//!   [`TaskComplexity`].
//!
//! # Design
//!
//! The crate is deliberately free of any HTTP client, provider SDK, async
//! runtime, or tool-execution concerns. It answers exactly one question:
//!
//! > given a task class (and optionally a complexity requirement), which
//! > model should I call, what is the fallback order, and what did it cost?
//!
//! Wiring the selected model to an actual API is the caller's job (see the
//! optional `genai` feature for a thin companion adapter to the
//! [`genai`](https://crates.io/crates/genai) crate).
//!
//! # Task-class mapping
//!
//! clawdius used coding-workflow-shaped task types. [`TaskClass`] generalizes
//! them; the suggested mapping is:
//!
//! | clawdius task type | [`TaskClass`] | typical model tier     |
//! |--------------------|---------------|------------------------|
//! | `Think`            | [`TaskClass::Fast`]    | cheap / fast  |
//! | `Summarize`        | [`TaskClass::Fast`]    | cheap / fast  |
//! | `Test`             | [`TaskClass::Fast`]    | cheap / fast  |
//! | `Plan`             | [`TaskClass::Balanced`]| mid tier      |
//! | `Review`           | [`TaskClass::Balanced`]| mid tier      |
//! | `Chat`             | [`TaskClass::Balanced`]| mid tier      |
//! | `Build`            | [`TaskClass::Power`]   | strongest     |
//! | *(embeddings)*     | [`TaskClass::Embedding`]| embedding models |
//!
//! # Feature flags
//!
//! - `serde` — `Serialize`/`Deserialize` on the public data types.
//! - `genai` — companion adapter from `genai::chat::ModelIden` to pricing
//!   entries (pulls in the `genai` crate; core stays genai-free).
//!
//! # Guarantees
//!
//! - `#![forbid(unsafe_code)]` — no unsafe anywhere in the crate.
//! - `#![deny(missing_docs)]` — the entire public API is documented.
//! - [`CostTracker::record`] checks the budget *before* mutating, so an
//!   over-budget request never lands: tracked spend can never exceed the
//!   limit (checked-then-added semantics).

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod cost;
mod error;
mod pricing;
mod routing;

#[cfg(feature = "genai")]
pub mod genai_compat;

pub use cost::{CostReport, CostTracker, ModelCostBreakdown};
pub use error::RouterError;
pub use pricing::{default_pricing_table, ModelPricing};
pub use routing::{RouteDecision, Router, RoutingRule, TaskClass, TaskComplexity};
