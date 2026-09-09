//! Companion adapter mapping [`genai`] model identifiers to pricing entries.
//!
//! This module is only compiled with the `genai` feature and exists so that
//! `genai` users can bridge into this crate without hand-writing name
//! plumbing. The core of the crate never references `genai`.

use crate::pricing::{default_pricing_table, ModelPricing};
use genai::adapter::AdapterKind;
use genai::ModelIden;

/// Build the canonical pricing-table key for a genai model identifier:
/// `"{adapter_kind}/{model_name}"`, e.g. `"OpenAI/gpt-4o"`.
pub fn pricing_key(iden: &ModelIden) -> String {
    format!(
        "{}/{}",
        iden.adapter_kind.as_str(),
        iden.model_name.as_str()
    )
}

/// Look up built-in pricing for a genai model identifier.
///
/// Resolution order:
/// 1. Exact key `"{adapter_kind}/{model_name}"` (e.g. `"OpenAI/gpt-4o"`).
/// 2. Exact model name alone (e.g. `"gpt-4o"`), regardless of adapter.
/// 3. A small alias table for popular models whose marketing names differ
///    from their identifiers (e.g. genai's `"claude-3-5-sonnet"` vs the
///    table's `"claude-3-5-sonnet-20241022"`).
///
/// Returns `None` when the model is unknown; callers can then estimate with
/// [`ModelPricing::default`] or add pricing via
/// [`crate::Router::add_pricing`].
pub fn default_pricing(iden: &ModelIden) -> Option<ModelPricing> {
    let table = default_pricing_table();
    let key = pricing_key(iden);
    if let Some(pricing) = table.get(&key) {
        return Some(pricing.clone());
    }
    if let Some(pricing) = table.get(iden.model_name.as_str()) {
        return Some(pricing.clone());
    }
    alias_model_key(iden.model_name.as_str()).and_then(|alias| table.get(alias).cloned())
}

/// Look up built-in pricing for a plain genai model-name string (e.g.
/// `"claude-3-5-sonnet"`, `"openai/gpt-4o"`, `"zai_coding::glm-4.6"`) by
/// resolving its adapter with [`AdapterKind::from_model`] first.
pub fn default_pricing_for_name(name: &str) -> Option<ModelPricing> {
    let adapter_kind = AdapterKind::from_model(name).ok()?;
    default_pricing(&ModelIden::new(adapter_kind, name))
}

/// Alias table mapping genai-style marketing names to built-in table keys.
fn alias_model_key(name: &str) -> Option<&'static str> {
    match name {
        "claude-sonnet-4" | "claude-sonnet-4-20250514" => Some("claude-sonnet-4-20250514"),
        "claude-3-5-sonnet" | "claude-3-5-sonnet-latest" => Some("claude-3-5-sonnet-20241022"),
        "claude-3-5-haiku" | "claude-3-5-haiku-latest" => Some("claude-3-5-haiku-20241022"),
        "claude-3-opus" | "claude-3-opus-latest" => Some("claude-3-opus-20240229"),
        "gemini-2.0-flash" => Some("gemini-2.0-flash"),
        "gemini-1.5-pro" => Some("gemini-1.5-pro"),
        "gpt-4o" => Some("gpt-4o"),
        "gpt-4o-mini" => Some("gpt-4o-mini"),
        "gpt-4-turbo" => Some("gpt-4-turbo"),
        "glm-4.6" => Some("glm-4.6"),
        "glm-5-turbo" => Some("glm-5-turbo"),
        "deepseek-chat" => Some("deepseek-chat"),
        "deepseek-coder" => Some("deepseek-coder"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)] // test assertions unwrap by design
    use super::*;
    use genai::adapter::AdapterKind;

    #[test]
    fn test_pricing_key_format() {
        let iden = ModelIden::new(AdapterKind::OpenAI, "gpt-4o");
        assert_eq!(pricing_key(&iden), "OpenAI/gpt-4o");
    }

    #[test]
    fn test_default_pricing_exact_model_name() {
        let iden = ModelIden::new(AdapterKind::Anthropic, "claude-sonnet-4-20250514");
        let pricing = default_pricing(&iden).expect("pricing for claude-sonnet-4");
        assert_eq!(pricing.input_per_1m, 3.0);
        assert_eq!(pricing.output_per_1m, 15.0);
    }

    #[test]
    fn test_default_pricing_alias() {
        let iden = ModelIden::new(AdapterKind::Anthropic, "claude-3-5-sonnet");
        let pricing = default_pricing(&iden).expect("aliased pricing");
        assert_eq!(pricing.context_window, 200_000);
    }

    #[test]
    fn test_default_pricing_unknown_is_none() {
        let iden = ModelIden::new(AdapterKind::Ollama, "totally-made-up-model");
        assert!(default_pricing(&iden).is_none());
    }
}
