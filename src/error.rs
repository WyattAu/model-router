//! Crate error type.

use crate::routing::TaskClass;
use thiserror::Error;

/// Errors returned by the routing core.
///
/// The core is deliberately small: the only runtime failure it can produce is
/// a budget rejection from [`crate::CostTracker::record`] — plus, with the
/// `json` feature enabled, pricing-table JSON load/serialization failures
/// from [`crate::PricingTable::from_json`] / [`crate::PricingTable::to_json`].
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RouterError {
    /// A spend record was rejected because it would push total tracked cost
    /// over the configured USD budget. The tracker is left unmodified.
    #[error(
        "budget exceeded: ${current:.4} recorded + ${attempted:.4} requested > ${limit:.2} limit"
    )]
    BudgetExceeded {
        /// Total spend already recorded, in USD.
        current: f64,
        /// Cost of the rejected record, in USD.
        attempted: f64,
        /// The configured budget limit, in USD.
        limit: f64,
    },
    /// No rule and no default model can satisfy a routing request.
    #[error("no route available for task class `{task}` and no default model configured")]
    NoRoute {
        /// The task class that could not be routed.
        task: TaskClass,
    },
    /// A pricing-table JSON payload could not be parsed or serialized
    /// (feature `json`). The message carries the underlying serde_json
    /// detail; the payload itself is not retained.
    #[error("pricing table JSON error: {0}")]
    Json(String),
}
