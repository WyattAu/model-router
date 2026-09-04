//! Per-model pricing data and a built-in table for popular models.

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
    /// output, tier 3).
    fn default() -> Self {
        Self {
            input_per_1m: 1.0,
            output_per_1m: 3.0,
            context_window: 128_000,
            max_output_tokens: 4_096,
            quality_tier: 3,
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
