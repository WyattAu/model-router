//! Per-model pricing data and a built-in table for popular models.
//!
//! Price tables go stale. [`ModelPricing`] carries an optional
//! `updated_at_unix` stamp and [`PricingTable`] can [`PricingTable::merge`]
//! fresher entries over older ones and load JSON snapshots
//! ([`PricingTable::from_json`], feature `json`), so rates can be refreshed
//! at runtime from a URL or file without a crate release.

#[cfg(feature = "json")]
use crate::error::RouterError;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-model pricing information.
///
/// Rates are expressed in USD per **1M tokens**, matching the convention used
/// by most provider pricing pages.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ModelPricing {
    /// Cost per 1M input tokens in USD.
    pub input_per_1m: f64,
    /// Cost per 1M output tokens in USD.
    pub output_per_1m: f64,
    /// Maximum context window in tokens.
    pub context_window: usize,
    /// Maximum output tokens.
    pub max_output_tokens: usize,
    /// Quality tier (1-5, 5 = highest). Used by
    /// [`crate::Router::select_by_complexity`] and [`ModelPricing::efficiency`].
    pub quality_tier: u8,
    /// Unix-seconds timestamp of when this entry was last refreshed from an
    /// external source (`None` = freshness unknown, e.g. built-in defaults).
    /// Used by [`PricingTable::merge`] conflict resolution. Deserialized as
    /// `None` when absent, so pre-0.1.1 JSON snapshots still load.
    #[cfg_attr(feature = "serde", serde(default))]
    updated_at_unix: Option<u64>,
}

impl ModelPricing {
    /// Create pricing from just the USD rates, filling the remaining fields
    /// with the same defaults as [`ModelPricing::default`] (128k context,
    /// 4,096 max output, quality tier 3).
    pub fn new(input_per_1m: f64, output_per_1m: f64) -> Self {
        Self {
            input_per_1m,
            output_per_1m,
            ..ModelPricing::default()
        }
    }

    /// Set the context window (builder style).
    pub fn with_context_window(mut self, context_window: usize) -> Self {
        self.context_window = context_window;
        self
    }

    /// Set the maximum output tokens (builder style).
    pub fn with_max_output_tokens(mut self, max_output_tokens: usize) -> Self {
        self.max_output_tokens = max_output_tokens;
        self
    }

    /// Set the quality tier, 1-5 (builder style).
    pub fn with_quality_tier(mut self, quality_tier: u8) -> Self {
        self.quality_tier = quality_tier;
        self
    }

    /// Stamp the entry with an explicit refresh time, as Unix seconds
    /// (builder style). See [`ModelPricing::updated_at_unix`].
    pub fn with_updated_at_unix(mut self, updated_at_unix: u64) -> Self {
        self.updated_at_unix = Some(updated_at_unix);
        self
    }

    /// Stamp the entry with the current wall-clock time as Unix seconds
    /// (builder style). Convenience for code that builds refresh snapshots.
    pub fn with_updated_at_now(self) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.with_updated_at_unix(now)
    }

    /// Unix-seconds timestamp of the last refresh from an external source,
    /// if known (`None` = freshness unknown).
    pub fn updated_at_unix(&self) -> Option<u64> {
        self.updated_at_unix
    }

    /// Whether `self` should replace `other` in a [`PricingTable::merge`]:
    /// true when `self` carries a strictly newer `updated_at_unix` stamp, or
    /// any stamp at all while `other` carries none. Two unstamped entries
    /// (or an older stamp) never win.
    pub fn is_newer_than(&self, other: &Self) -> bool {
        match (self.updated_at_unix, other.updated_at_unix) {
            (Some(mine), Some(theirs)) => mine > theirs,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// Calculate the cost in USD for a given number of input/output tokens.
    pub fn cost(&self, input_tokens: usize, output_tokens: usize) -> f64 {
        (input_tokens as f64 * self.input_per_1m / 1_000_000.0)
            + (output_tokens as f64 * self.output_per_1m / 1_000_000.0)
    }

    /// Cost efficiency: quality tier per average-USD cost.
    ///
    /// Higher is better. Free models (both rates zero) return [`f64::MAX`].
    pub fn efficiency(&self) -> f64 {
        let avg_cost = (self.input_per_1m + self.output_per_1m) / 2.0;
        if avg_cost > 0.0 {
            self.quality_tier as f64 / avg_cost
        } else {
            f64::MAX
        }
    }
}

impl Default for ModelPricing {
    /// Placeholder pricing used to *estimate* cost for models that are not in
    /// the table ($1.00/1M input, $3.00/1M output, 128k context, 4,096 max
    /// output, tier 3, no freshness stamp).
    fn default() -> Self {
        Self {
            input_per_1m: 1.0,
            output_per_1m: 3.0,
            context_window: 128_000,
            max_output_tokens: 4_096,
            quality_tier: 3,
            updated_at_unix: None,
        }
    }
}

/// Built-in pricing table for popular models.
///
/// Rates reflect publicly listed prices at the time of extraction (May 2025)
/// and can be overridden at runtime with
/// [`crate::Router::add_pricing`]. Keys are plain model identifiers; namespaced
/// keys such as `openrouter/google/gemma-3-4b-it:free` are kept verbatim.
///
/// The table is a [`BTreeMap`], so iteration — and therefore
/// [`crate::Router::select_by_complexity`] tie-breaking — is deterministic.
pub fn default_pricing_table() -> BTreeMap<String, ModelPricing> {
    let mut table = BTreeMap::new();

    let mut insert = |key: &str, pricing: ModelPricing| {
        table.insert(key.to_string(), pricing);
    };

    // Claude models (Anthropic)
    insert(
        "claude-sonnet-4-20250514",
        ModelPricing::new(3.0, 15.0)
            .with_context_window(200_000)
            .with_max_output_tokens(16_384)
            .with_quality_tier(5),
    );
    insert(
        "claude-3-5-sonnet-20241022",
        ModelPricing::new(3.0, 15.0)
            .with_context_window(200_000)
            .with_max_output_tokens(8_192)
            .with_quality_tier(4),
    );
    insert(
        "claude-3-5-haiku-20241022",
        ModelPricing::new(0.8, 4.0)
            .with_context_window(200_000)
            .with_max_output_tokens(8_192)
            .with_quality_tier(3),
    );
    insert(
        "claude-3-opus-20240229",
        ModelPricing::new(15.0, 75.0)
            .with_context_window(200_000)
            .with_max_output_tokens(4_096)
            .with_quality_tier(5),
    );

    // GPT models (OpenAI)
    insert(
        "gpt-4o",
        ModelPricing::new(2.5, 10.0)
            .with_context_window(128_000)
            .with_max_output_tokens(16_384)
            .with_quality_tier(4),
    );
    insert(
        "gpt-4o-mini",
        ModelPricing::new(0.15, 0.6)
            .with_context_window(128_000)
            .with_max_output_tokens(16_384)
            .with_quality_tier(3),
    );
    insert(
        "gpt-4-turbo",
        ModelPricing::new(10.0, 30.0)
            .with_context_window(128_000)
            .with_max_output_tokens(4_096)
            .with_quality_tier(5),
    );

    // Gemini models (Google)
    insert(
        "gemini-2.0-flash",
        ModelPricing::new(0.1, 0.4)
            .with_context_window(1_000_000)
            .with_max_output_tokens(8_192)
            .with_quality_tier(3),
    );
    insert(
        "gemini-1.5-pro",
        ModelPricing::new(1.25, 5.0)
            .with_context_window(2_000_000)
            .with_max_output_tokens(8_192)
            .with_quality_tier(4),
    );

    // GLM models (ZAI)
    insert(
        "glm-4.6",
        ModelPricing::new(0.5, 0.5)
            .with_context_window(128_000)
            .with_max_output_tokens(4_096)
            .with_quality_tier(3),
    );
    insert(
        "glm-5-turbo",
        ModelPricing::new(0.5, 0.5)
            .with_context_window(128_000)
            .with_max_output_tokens(4_096)
            .with_quality_tier(3),
    );

    // DeepSeek
    insert(
        "deepseek-chat",
        ModelPricing::new(0.14, 0.28)
            .with_context_window(64_000)
            .with_max_output_tokens(8_192)
            .with_quality_tier(3),
    );
    insert(
        "deepseek-coder",
        ModelPricing::new(0.14, 0.28)
            .with_context_window(64_000)
            .with_max_output_tokens(8_192)
            .with_quality_tier(3),
    );

    // OpenRouter prefixed models
    insert(
        "anthropic/claude-3.5-sonnet",
        ModelPricing::new(3.0, 15.0)
            .with_context_window(200_000)
            .with_max_output_tokens(8_192)
            .with_quality_tier(4),
    );
    insert(
        "openai/gpt-4o",
        ModelPricing::new(2.5, 10.0)
            .with_context_window(128_000)
            .with_max_output_tokens(16_384)
            .with_quality_tier(4),
    );
    insert(
        "google/gemma-3-4b-it:free",
        ModelPricing::new(0.0, 0.0)
            .with_context_window(32_000)
            .with_max_output_tokens(4_096)
            .with_quality_tier(2),
    );
    insert(
        "openai/gpt-oss-20b:free",
        ModelPricing::new(0.0, 0.0)
            .with_context_window(32_000)
            .with_max_output_tokens(4_096)
            .with_quality_tier(2),
    );

    table
}

/// A refreshable set of [`ModelPricing`] entries keyed by model identifier.
///
/// The built-in rates returned by [`default_pricing_table`] inevitably go
/// stale. [`PricingTable`] exists so callers can refresh them at runtime
/// (from a file, an HTTP JSON endpoint, etc.) without waiting for a crate
/// release:
///
/// 1. Serialize a table with the `serde`/`json` features and host the JSON
///    (the shape is a plain object mapping model id to pricing fields).
/// 2. Load it with [`PricingTable::from_json`] (feature `json`).
/// 3. [`PricingTable::merge`] it over your current table — entries that are
///    missing or stamped newer (see [`ModelPricing::is_newer_than`]) win;
///    everything else is left untouched.
///
/// The built-in defaults are *unstamped* (`updated_at_unix == None`), so any
/// timestamped refresh entry wins over them.
///
/// With the `serde` feature the table (de)serializes transparently as a
/// plain `{ "model-id": {...} }` JSON object.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize), serde(transparent))]
pub struct PricingTable {
    entries: BTreeMap<String, ModelPricing>,
}

impl PricingTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// A table seeded with the built-in rates (see [`default_pricing_table`]).
    pub fn with_defaults() -> Self {
        Self {
            entries: default_pricing_table(),
        }
    }

    /// Insert (or replace) pricing for a model key, returning the previous
    /// entry if any. Insertions are not timestamp-checked; use
    /// [`PricingTable::merge`] for freshness-aware upserts.
    pub fn insert(
        &mut self,
        model: impl Into<String>,
        pricing: ModelPricing,
    ) -> Option<ModelPricing> {
        self.entries.insert(model.into(), pricing)
    }

    /// Pricing for a model key, if present.
    pub fn get(&self, model: &str) -> Option<&ModelPricing> {
        self.entries.get(model)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate `(model id, pricing)` pairs in deterministic (sorted) order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ModelPricing)> {
        self.entries
            .iter()
            .map(|(model, pricing)| (model.as_str(), pricing))
    }

    /// Upsert every entry of `other` into `self`, keeping `self`'s entry
    /// unless `other`'s is missing from `self` or carries a strictly newer
    /// [`ModelPricing::updated_at_unix`] stamp.
    ///
    /// Semantics:
    ///
    /// - key only in `other` → added;
    /// - key in both, `other` newer (or `self` unstamped, `other` stamped) →
    ///   `other`'s entry replaces `self`'s;
    /// - key in both, `self` equal-or-newer (or both unstamped) → `self`'s
    ///   entry is preserved.
    pub fn merge(&mut self, other: &PricingTable) {
        merge_into(&mut self.entries, other);
    }

    /// Parse a table from JSON (feature `json`).
    ///
    /// Accepts the same shape [`PricingTable::to_json`] emits: a plain object
    /// mapping model id to a pricing object. The `updated_at_unix` field is
    /// optional (`None` when absent), so snapshots produced before 0.1.1 —
    /// or by external tooling — still load.
    ///
    /// # Examples
    ///
    /// ```
    /// use model_router::PricingTable;
    ///
    /// let json = r#"{
    ///     "gpt-4o": {
    ///         "input_per_1m": 2.5,
    ///         "output_per_1m": 10.0,
    ///         "context_window": 128000,
    ///         "max_output_tokens": 16384,
    ///         "quality_tier": 4,
    ///         "updated_at_unix": 1757289600
    ///     }
    /// }"#;
    /// let table = PricingTable::from_json(json).expect("valid table JSON");
    /// assert_eq!(table.get("gpt-4o").expect("entry").updated_at_unix(), Some(1_757_289_600));
    /// ```
    #[cfg(feature = "json")]
    pub fn from_json(json: &str) -> Result<Self, RouterError> {
        serde_json::from_str(json).map_err(|e| RouterError::Json(e.to_string()))
    }

    /// Serialize the table to JSON (feature `json`).
    ///
    /// The output is a plain object mapping model id to pricing object, with
    /// `updated_at_unix` present when stamped — the exact shape
    /// [`PricingTable::from_json`] accepts, suitable for hosting at a URL or
    /// writing to a file for the refresh workflow.
    #[cfg(feature = "json")]
    pub fn to_json(&self) -> Result<String, RouterError> {
        serde_json::to_string(self).map_err(|e| RouterError::Json(e.to_string()))
    }
}

/// Shared upsert used by [`PricingTable::merge`] and
/// [`crate::Router::merge_pricing`].
pub(crate) fn merge_into(entries: &mut BTreeMap<String, ModelPricing>, other: &PricingTable) {
    for (model, incoming) in other.iter() {
        match entries.get(model) {
            Some(existing) if !incoming.is_newer_than(existing) => {}
            _ => {
                entries.insert(model.to_string(), incoming.clone());
            }
        }
    }
}
