//! USD cost tracking with budget enforcement.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::RouterError;
use crate::pricing::ModelPricing;

/// Fixed-point scale for stored costs: units of $0.0001 (i.e. USD * 10,000).
const COST_SCALE: f64 = 10_000.0;

/// Accumulates token usage and USD spend, optionally against a budget.
///
/// * Totals are stored in fixed point (units of $0.0001) in atomics, so
///   [`CostTracker::total_usd`], [`CostTracker::total_input_tokens`] and
///   [`CostTracker::total_output_tokens`] are lock-free and `Send + Sync`.
/// * The per-model breakdown is guarded by a [`std::sync::Mutex`].
/// * Cloning a tracker shares the same underlying state (all fields are
///   `Arc`-backed), so all clones observe the same spend.
///
/// # Budget semantics
///
/// [`CostTracker::record`] checks the budget *before* mutating any state. A
/// record that would push total spend past the limit is rejected with
/// [`RouterError::BudgetExceeded`] and nothing is accumulated. Therefore, as
/// long as every call is gated through `record`, tracked spend can never
/// exceed the limit.
///
/// # Example
///
/// ```
/// use model_router::{CostTracker, ModelPricing};
///
/// let tracker = CostTracker::new(Some(0.01)); // $0.01 budget
/// let pricing = ModelPricing::new(3.0, 15.0); // Claude Sonnet class
///
/// tracker.record("claude-sonnet-4", 100, 50, &pricing).unwrap();
/// assert!(tracker.total_usd() < 0.01);
/// assert!(!tracker.is_over_budget());
/// ```
#[derive(Debug, Clone)]
pub struct CostTracker {
    /// Total cost, stored as USD * [`COST_SCALE`] in fixed point.
    total_cost: Arc<AtomicU64>,
    /// Total input tokens observed.
    total_input_tokens: Arc<AtomicU64>,
    /// Total output tokens observed.
    total_output_tokens: Arc<AtomicU64>,
    /// Per-model cost breakdown.
    per_model: Arc<Mutex<BTreeMap<String, ModelCostRecord>>>,
    /// Budget limit in USD (`None` = unlimited).
    budget_limit_usd: Option<f64>,
}

/// Internal per-model accumulation record (fixed point, see [`COST_SCALE`]).
#[derive(Debug, Clone, Default)]
struct ModelCostRecord {
    input_tokens: u64,
    output_tokens: u64,
    cost_fixed: u64,
    request_count: u64,
}

/// Per-model cost breakdown for reporting.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModelCostBreakdown {
    /// Total input tokens attributed to this model.
    pub input_tokens: u64,
    /// Total output tokens attributed to this model.
    pub output_tokens: u64,
    /// Total attributed cost in USD.
    pub cost_usd: f64,
    /// Number of `record` calls accepted for this model.
    pub request_count: u64,
}

/// Snapshot of all tracked usage, including budget status.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CostReport {
    /// Total recorded spend in USD.
    pub total_cost_usd: f64,
    /// Total input tokens recorded.
    pub total_input_tokens: u64,
    /// Total output tokens recorded.
    pub total_output_tokens: u64,
    /// Spend broken down per model, keyed by model name.
    pub per_model: BTreeMap<String, ModelCostBreakdown>,
    /// Configured budget limit, if any.
    pub budget_limit_usd: Option<f64>,
    /// Whether total spend has reached or exceeded the budget.
    pub is_over_budget: bool,
}

impl CostTracker {
    /// Create a new cost tracker, optionally with a USD budget limit.
    pub fn new(budget_limit_usd: Option<f64>) -> Self {
        Self {
            total_cost: Arc::new(AtomicU64::new(0)),
            total_input_tokens: Arc::new(AtomicU64::new(0)),
            total_output_tokens: Arc::new(AtomicU64::new(0)),
            per_model: Arc::new(Mutex::new(BTreeMap::new())),
            budget_limit_usd,
        }
    }

    /// The configured budget limit in USD, if any.
    pub fn budget_limit_usd(&self) -> Option<f64> {
        self.budget_limit_usd
    }

    /// Record token usage for `model`, priced with `pricing`.
    ///
    /// Fails with [`RouterError::BudgetExceeded`] (without mutating any
    /// state) if the record would push total spend past the budget limit.
    ///
    /// # Precision
    ///
    /// Costs are accumulated in fixed point at $0.0001 granularity and
    /// *truncated* into that grid, so a recorded delta never exceeds the
    /// true cost and tracked totals err slightly low. Costs under $0.0001
    /// (a hundredth of a cent) truncate to zero. The budget pre-check itself
    /// runs in `f64`, so enforcement precision is unaffected.
    pub fn record(
        &self,
        model: &str,
        input_tokens: usize,
        output_tokens: usize,
        pricing: &ModelPricing,
    ) -> Result<(), RouterError> {
        let cost_usd = pricing.cost(input_tokens, output_tokens);
        let cost_fixed = (cost_usd * COST_SCALE) as u64;

        if let Some(limit) = self.budget_limit_usd {
            let current = self.total_usd();
            if current + cost_usd > limit {
                return Err(RouterError::BudgetExceeded {
                    current,
                    attempted: cost_usd,
                    limit,
                });
            }
        }

        self.total_cost.fetch_add(cost_fixed, Ordering::Relaxed);
        self.total_input_tokens
            .fetch_add(input_tokens as u64, Ordering::Relaxed);
        self.total_output_tokens
            .fetch_add(output_tokens as u64, Ordering::Relaxed);

        // Justified: poisoning implies a panic mid-update; tracker is best-effort.
        #[allow(clippy::expect_used)]
        let mut per_model = self.per_model.lock().expect("cost tracker mutex poisoned");
        let record = per_model.entry(model.to_string()).or_default();
        record.input_tokens += input_tokens as u64;
        record.output_tokens += output_tokens as u64;
        record.cost_fixed += cost_fixed;
        record.request_count += 1;

        Ok(())
    }

    /// Total recorded cost in USD.
    pub fn total_usd(&self) -> f64 {
        self.total_cost.load(Ordering::Relaxed) as f64 / COST_SCALE
    }

    /// Total input tokens recorded across all models.
    pub fn total_input_tokens(&self) -> u64 {
        self.total_input_tokens.load(Ordering::Relaxed)
    }

    /// Total output tokens recorded across all models.
    pub fn total_output_tokens(&self) -> u64 {
        self.total_output_tokens.load(Ordering::Relaxed)
    }

    /// Unspent budget in USD (`None` when no budget is configured). Never
    /// negative; zero once the budget is exhausted.
    pub fn remaining_usd(&self) -> Option<f64> {
        self.budget_limit_usd
            .map(|limit| (limit - self.total_usd()).max(0.0))
    }

    /// Per-model cost breakdown.
    pub fn per_model_breakdown(&self) -> BTreeMap<String, ModelCostBreakdown> {
        #[allow(clippy::expect_used)]
        let per_model = self.per_model.lock().expect("cost tracker mutex poisoned");
        per_model
            .iter()
            .map(|(model, record)| {
                (
                    model.clone(),
                    ModelCostBreakdown {
                        input_tokens: record.input_tokens,
                        output_tokens: record.output_tokens,
                        cost_usd: record.cost_fixed as f64 / COST_SCALE,
                        request_count: record.request_count,
                    },
                )
            })
            .collect()
    }

    /// Whether total spend has reached or exceeded the budget limit.
    ///
    /// Always `false` when no budget is configured.
    pub fn is_over_budget(&self) -> bool {
        match self.budget_limit_usd {
            Some(limit) => self.total_usd() >= limit,
            None => false,
        }
    }

    /// Build a [`CostReport`] snapshot of the current state.
    pub fn report(&self) -> CostReport {
        CostReport {
            total_cost_usd: self.total_usd(),
            total_input_tokens: self.total_input_tokens(),
            total_output_tokens: self.total_output_tokens(),
            per_model: self.per_model_breakdown(),
            budget_limit_usd: self.budget_limit_usd,
            is_over_budget: self.is_over_budget(),
        }
    }
}
