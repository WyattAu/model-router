//! Task-class routing with fallback chains.

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

use crate::pricing::{default_pricing_table, merge_into, ModelPricing, PricingTable};

/// Generic routing class for a unit of work.
///
/// This replaces clawdius's coding-workflow-shaped `TaskType`. The intended
/// mapping (also enforced by [`TaskClass::for_phase_name`] and documented in
/// the crate root):
///
/// - `Think`, `Test`, `Summarize` → [`TaskClass::Fast`]
/// - `Plan`, `Review`, `Chat` → [`TaskClass::Balanced`]
/// - `Build` → [`TaskClass::Power`]
/// - embedding work → [`TaskClass::Embedding`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(
    feature = "serde",
    derive(Serialize, Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum TaskClass {
    /// Low-stakes work that should use the cheapest acceptable model
    /// (thinking, testing, summarization).
    Fast,
    /// Everyday work handled well by a mid-tier model (planning, review,
    /// general chat).
    Balanced,
    /// High-stakes generation that needs the strongest available model
    /// (code generation, architecture).
    Power,
    /// Embedding work; routed to an embedding model, never a chat model.
    Embedding,
}

impl TaskClass {
    /// Map a phase/step name to a task class.
    ///
    /// Accepts the clawdius phase vocabulary (`think`, `plan`, `build`,
    /// `implement`, `code`, `test`, `verify`, `review`, `reflect`,
    /// `summarize`) plus `embed`/`embedding(s)` and `chat`; matching is
    /// case-insensitive. Unknown names map to [`TaskClass::Balanced`].
    pub fn for_phase_name(phase: &str) -> Self {
        match phase.to_lowercase().as_str() {
            "think" | "test" | "verify" | "summarize" | "summarise" => Self::Fast,
            "embed" | "embedding" | "embeddings" => Self::Embedding,
            "build" | "implement" | "code" => Self::Power,
            _ => Self::Balanced,
        }
    }
}

impl std::fmt::Display for TaskClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fast => write!(f, "fast"),
            Self::Balanced => write!(f, "balanced"),
            Self::Power => write!(f, "power"),
            Self::Embedding => write!(f, "embedding"),
        }
    }
}

/// Task complexity levels for cost-aware routing.
///
/// Each level implies a minimum [`ModelPricing::quality_tier`] when
/// selecting a model with [`Router::select_by_complexity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaskComplexity {
    /// Simple tasks: formatting, renaming, simple refactors (tier >= 2).
    Simple,
    /// Medium tasks: bug fixes, small features, code review (tier >= 3).
    Medium,
    /// Complex tasks: architecture, multi-file refactors, complex
    /// algorithms (tier >= 4).
    Complex,
    /// Critical tasks: security fixes, production issues, complex
    /// debugging (tier >= 5).
    Critical,
}

impl TaskComplexity {
    /// Minimum [`ModelPricing::quality_tier`] required for this complexity.
    pub fn min_quality_tier(self) -> u8 {
        match self {
            Self::Simple => 2,
            Self::Medium => 3,
            Self::Complex => 4,
            Self::Critical => 5,
        }
    }
}

/// A routing rule: which model to use for a [`TaskClass`], with an optional
/// fallback rule tried if the primary is unavailable.
///
/// The provider concept from clawdius was dropped from the core: a model key
/// is an opaque string that must match an entry in the pricing table (or be
/// estimated with placeholder pricing). Namespaced keys such as
/// `openrouter/google/gemma-3-4b-it:free` work fine.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RoutingRule {
    /// Task class this rule applies to.
    pub task: TaskClass,
    /// Model key (e.g. `"claude-sonnet-4-20250514"`).
    pub model: String,
    /// Fallback rule tried if the primary model is unavailable.
    pub fallback: Option<Box<RoutingRule>>,
}

impl RoutingRule {
    /// Create a rule with no fallback.
    pub fn new(task: TaskClass, model: impl Into<String>) -> Self {
        Self {
            task,
            model: model.into(),
            fallback: None,
        }
    }

    /// Attach a fallback rule (builder style).
    pub fn with_fallback(mut self, fallback: RoutingRule) -> Self {
        self.fallback = Some(Box::new(fallback));
        self
    }

    /// Iterate this rule followed by its fallback chain, in order.
    pub fn chain(&self) -> impl Iterator<Item = &RoutingRule> {
        let mut current = Some(self);
        std::iter::from_fn(move || {
            let rule = current?;
            current = rule.fallback.as_deref();
            Some(rule)
        })
    }
}

/// The result of routing a task: the primary model plus its fallback order.
///
/// `primary` is always `fallbacks[0]`'s model when a rule matched (i.e.
/// [`RouteDecision::candidates`] yields the exact chain order); it is the
/// router's default model when no rule matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision<'a> {
    /// The task class that was routed.
    pub task: TaskClass,
    /// Selected primary model.
    pub primary: &'a str,
    /// Fallback chain in try order: primary first, then each fallback.
    pub fallbacks: Vec<&'a str>,
}

impl<'a> RouteDecision<'a> {
    /// All candidate models in try order (primary, then fallbacks).
    pub fn candidates(&self) -> impl Iterator<Item = &'a str> + '_ {
        std::iter::once(self.primary).chain(self.fallbacks.iter().copied().skip(1))
    }
}

/// Cost-aware task router.
///
/// Pure decision logic: given a [`TaskClass`] it resolves the configured
/// [`RoutingRule`] (or the default model) and returns the fallback chain;
/// given a [`TaskComplexity`] it picks the most cost-efficient model whose
/// quality tier meets the requirement. All selection is deterministic —
/// the pricing table is a [`BTreeMap`] and sorting is stable, so ties are
/// broken by model name.
///
/// # Example
///
/// ```
/// use model_router::{Router, RoutingRule, TaskClass};
///
/// let mut router = Router::new("claude-sonnet-4-20250514");
/// router.add_rule(
///     RoutingRule::new(TaskClass::Fast, "claude-3-5-haiku-20241022")
///         .with_fallback(RoutingRule::new(TaskClass::Fast, "gpt-4o-mini")),
/// );
///
/// let decision = router.route(TaskClass::Fast);
/// assert_eq!(decision.primary, "claude-3-5-haiku-20241022");
/// assert_eq!(
///     decision.candidates().collect::<Vec<_>>(),
///     ["claude-3-5-haiku-20241022", "gpt-4o-mini"]
/// );
/// ```
#[derive(Debug, Clone)]
pub struct Router {
    /// Routing rules keyed by task class.
    rules: HashMap<TaskClass, RoutingRule>,
    /// Default model when no rule matches.
    default_model: String,
    /// Pricing table (BTreeMap => deterministic iteration order).
    pricing: BTreeMap<String, ModelPricing>,
}

impl Router {
    /// Create a router with the built-in pricing table and the given default
    /// model (used whenever a task class has no rule).
    pub fn new(default_model: impl Into<String>) -> Self {
        Self {
            rules: HashMap::new(),
            default_model: default_model.into(),
            pricing: default_pricing_table(),
        }
    }

    /// Set the default model (used when no rule matches a task class).
    pub fn set_default_model(&mut self, model: impl Into<String>) {
        self.default_model = model.into();
    }

    /// Add (or replace) the rule for a task class.
    pub fn add_rule(&mut self, rule: RoutingRule) {
        self.rules.insert(rule.task, rule);
    }

    /// Builder-style version of [`Router::add_rule`].
    pub fn with_rule(mut self, rule: RoutingRule) -> Self {
        self.add_rule(rule);
        self
    }

    /// Add (or replace) pricing for a model key.
    pub fn add_pricing(&mut self, model: impl Into<String>, pricing: ModelPricing) {
        self.pricing.insert(model.into(), pricing);
    }

    /// Builder-style version of [`Router::add_pricing`].
    pub fn with_pricing(mut self, model: impl Into<String>, pricing: ModelPricing) -> Self {
        self.add_pricing(model, pricing);
        self
    }

    /// Freshness-aware merge of a [`PricingTable`] into the router's table
    /// (see [`PricingTable::merge`]): entries that are missing from the
    /// router's table or stamped with a newer
    /// [`ModelPricing::updated_at_unix`] replace the existing ones; every
    /// other entry is left untouched. This is the refresh path for keeping
    /// built-in rates current without a crate release.
    pub fn merge_pricing(&mut self, table: &PricingTable) {
        merge_into(&mut self.pricing, table);
    }

    /// The current default model.
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// Pricing for a model key, if present in the table.
    pub fn pricing_for(&self, model: &str) -> Option<&ModelPricing> {
        self.pricing.get(model)
    }

    /// The full pricing table.
    pub fn pricing_table(&self) -> &BTreeMap<String, ModelPricing> {
        &self.pricing
    }

    /// The rule configured for a task class, if any.
    pub fn rule_for(&self, task: TaskClass) -> Option<&RoutingRule> {
        self.rules.get(&task)
    }

    /// Route a task class: resolve its rule (or the default model) and
    /// return the primary model plus the full fallback chain in try order.
    ///
    /// Deterministic: the same configuration always yields the same decision.
    pub fn route(&self, task: TaskClass) -> RouteDecision<'_> {
        match self.rules.get(&task) {
            Some(rule) => {
                let fallbacks = rule.chain().map(|r| r.model.as_str()).collect();
                RouteDecision {
                    task,
                    primary: rule.model.as_str(),
                    fallbacks,
                }
            }
            None => RouteDecision {
                task,
                primary: &self.default_model,
                fallbacks: Vec::new(),
            },
        }
    }

    /// Select the most cost-efficient model whose quality tier meets the
    /// complexity requirement.
    ///
    /// Returns `None` when no priced model meets the minimum tier (callers
    /// can fall back to [`Router::default_model`] in that case).
    pub fn select_by_complexity(&self, complexity: TaskComplexity) -> Option<&str> {
        let min_quality = complexity.min_quality_tier();
        let mut candidates: Vec<(&String, &ModelPricing)> = self
            .pricing
            .iter()
            .filter(|(_, p)| p.quality_tier >= min_quality)
            .collect();
        // Stable sort on a deterministically ordered BTreeMap: ties resolve
        // to the alphabetically first model name.
        candidates.sort_by(|a, b| {
            b.1.efficiency()
                .partial_cmp(&a.1.efficiency())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.first().map(|(name, _)| name.as_str())
    }

    /// Estimate the USD cost for a model and token counts.
    ///
    /// Models missing from the pricing table are estimated with the
    /// documented placeholder pricing (see [`ModelPricing::default`]).
    pub fn estimate_cost(&self, model: &str, input_tokens: usize, output_tokens: usize) -> f64 {
        self.pricing
            .get(model)
            .unwrap_or(&ModelPricing::default())
            .cost(input_tokens, output_tokens)
    }

    /// Build a default rule set for a typical setup: the given provider/model
    /// as the strong "primary" model and a known cheaper model from the same
    /// provider for low-stakes work.
    ///
    /// Produces: `Fast` → cheap model, `Balanced` → primary, `Power` →
    /// primary. Embedding work is left unconfigured (add a rule explicitly).
    ///
    /// Provider names follow clawdius's: `anthropic`, `openai`, `google`,
    /// `zai`, `openrouter`. Unknown providers fall back to using the primary
    /// model for every class.
    pub fn default_rules(provider: &str, primary_model: &str) -> Vec<RoutingRule> {
        let cheap_model = match provider {
            "anthropic" => "claude-3-5-haiku-20241022",
            "openai" => "gpt-4o-mini",
            "google" => "gemini-2.0-flash",
            "zai" => "glm-4.6",
            "openrouter" => "google/gemma-3-4b-it:free",
            _ => primary_model,
        };

        vec![
            RoutingRule::new(TaskClass::Fast, cheap_model),
            RoutingRule::new(TaskClass::Balanced, primary_model),
            RoutingRule::new(TaskClass::Power, primary_model),
        ]
    }

    /// Create a router configured with [`Router::default_rules`] for the
    /// given provider/model, plus the built-in pricing table.
    pub fn with_default_rules(provider: &str, primary_model: impl Into<String>) -> Self {
        let primary_model = primary_model.into();
        let mut router = Router::new(primary_model.clone());
        for rule in Router::default_rules(provider, &primary_model) {
            router.add_rule(rule);
        }
        router
    }
}
