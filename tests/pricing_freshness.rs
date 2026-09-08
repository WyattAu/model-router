//! Price-table freshness: `updated_at_unix` stamps, `PricingTable::merge`
//! semantics, and the JSON refresh workflow (feature `json`).

use std::time::{SystemTime, UNIX_EPOCH};

use model_router::{ModelPricing, PricingTable, Router};

/// Pricing with rate `input`/`3*input`, no stamp.
fn p(input: f64) -> ModelPricing {
    ModelPricing::new(input, input * 3.0)
}

/// Pricing with rate `input`/`3*input`, stamped at `stamp` Unix seconds.
fn stamped(input: f64, stamp: u64) -> ModelPricing {
    p(input).with_updated_at_unix(stamp)
}

// --- updated_at_unix basics ---------------------------------------------------

#[test]
fn test_updated_at_getter_and_builder() {
    let unstamped = p(1.0);
    assert_eq!(unstamped.updated_at_unix(), None);

    let stamped_entry = p(1.0).with_updated_at_unix(1_757_289_600);
    assert_eq!(stamped_entry.updated_at_unix(), Some(1_757_289_600));
}

#[test]
fn test_with_updated_at_now_stamps_wall_clock() {
    let before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let entry = p(1.0).with_updated_at_now();
    let after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let stamp = entry.updated_at_unix().expect("now-stamped entry");
    assert!(
        (before..=after).contains(&stamp),
        "{stamp} not in [{before}, {after}]"
    );
}

#[test]
fn test_is_newer_than_semantics() {
    let fresh = stamped(1.0, 200);
    let stale = stamped(2.0, 100);
    let undated = p(3.0);

    // Strictly newer wins; equal stamps do not.
    assert!(fresh.is_newer_than(&stale));
    assert!(!stale.is_newer_than(&fresh));
    assert!(!fresh.is_newer_than(&stamped(9.9, 200)));

    // Any stamp beats no stamp; no stamp never wins.
    assert!(fresh.is_newer_than(&undated));
    assert!(!undated.is_newer_than(&fresh));
    assert!(!undated.is_newer_than(&undated));
}

// --- merge semantics -----------------------------------------------------------

#[test]
fn test_merge_adds_missing_entries() {
    let mut table = PricingTable::new();
    table.insert("a", p(1.0));

    let mut fresh = PricingTable::new();
    fresh.insert("b", stamped(2.0, 100));

    table.merge(&fresh);
    assert_eq!(table.len(), 2);
    assert_eq!(table.get("b").expect("b added").input_per_1m, 2.0);
    assert_eq!(table.get("a").expect("a untouched").input_per_1m, 1.0);
}

#[test]
fn test_merge_newer_entry_wins() {
    let mut table = PricingTable::new();
    table.insert("a", stamped(1.0, 100));

    let mut fresh = PricingTable::new();
    fresh.insert("a", stamped(2.0, 200));

    table.merge(&fresh);
    assert_eq!(table.get("a").expect("entry").input_per_1m, 2.0);
    assert_eq!(table.get("a").expect("entry").updated_at_unix(), Some(200));
}

#[test]
fn test_merge_stale_entry_preserved() {
    // self holds the newer entry; the older incoming one must not clobber it.
    let mut table = PricingTable::new();
    table.insert("a", stamped(1.0, 300));

    let mut stale = PricingTable::new();
    stale.insert("a", stamped(9.9, 200));

    table.merge(&stale);
    assert_eq!(table.get("a").expect("entry").input_per_1m, 1.0);
    assert_eq!(table.get("a").expect("entry").updated_at_unix(), Some(300));
}

#[test]
fn test_merge_undated_never_overwrites() {
    // Unstamped incoming entries never replace anything, stamped or not.
    let mut table = PricingTable::new();
    table.insert("stamped", stamped(1.0, 100));
    table.insert("undated", p(2.0));

    let mut incoming = PricingTable::new();
    incoming.insert("stamped", p(9.9));
    incoming.insert("undated", p(9.9));

    table.merge(&incoming);
    assert_eq!(table.get("stamped").expect("entry").input_per_1m, 1.0);
    assert_eq!(table.get("undated").expect("entry").input_per_1m, 2.0);
}

#[test]
fn test_merge_stamped_wins_over_undated_self() {
    let mut table = PricingTable::new();
    table.insert("a", p(1.0)); // built-in style: no stamp

    let mut fresh = PricingTable::new();
    fresh.insert("a", stamped(2.0, 1_757_289_600));

    table.merge(&fresh);
    assert_eq!(table.get("a").expect("entry").input_per_1m, 2.0);
}

#[test]
fn test_merge_self_only_entries_kept() {
    let mut table = PricingTable::new();
    table.insert("a", stamped(1.0, 100));
    table.insert("b", p(2.0));

    let mut fresh = PricingTable::new();
    fresh.insert("c", stamped(3.0, 100));

    table.merge(&fresh);
    assert_eq!(table.len(), 3);
    assert!(table.get("a").is_some() && table.get("b").is_some() && table.get("c").is_some());
}

#[test]
fn test_merge_with_defaults_refresh_workflow() {
    // Built-in defaults are unstamped, so any stamped refresh wins.
    let mut table = PricingTable::with_defaults();
    assert!(!table.is_empty());
    assert_eq!(
        table
            .get("gpt-4o")
            .expect("built-in entry")
            .updated_at_unix(),
        None
    );

    let mut fresh = PricingTable::new();
    fresh.insert("gpt-4o", stamped(0.0, 1_757_289_600)); // hypothetical price drop
    fresh.insert("brand-new-model", stamped(0.1, 1_757_289_600));

    table.merge(&fresh);
    assert_eq!(table.get("gpt-4o").expect("entry").input_per_1m, 0.0);
    assert!(table.get("brand-new-model").is_some());
    // Unrelated built-ins stay put.
    assert_eq!(table.get("glm-4.6").expect("entry").input_per_1m, 0.5);
}

// --- Router::merge_pricing ------------------------------------------------------

#[test]
fn test_router_merge_pricing() {
    let mut router = Router::new("claude-sonnet-4-20250514");
    assert_eq!(
        router.pricing_for("gpt-4o").expect("built-in").input_per_1m,
        2.5
    );

    let mut fresh = PricingTable::new();
    fresh.insert("gpt-4o", stamped(0.0, 1_757_289_600));
    fresh.insert("new-model", p(0.1));
    router.merge_pricing(&fresh);

    assert_eq!(
        router
            .pricing_for("gpt-4o")
            .expect("refreshed")
            .input_per_1m,
        0.0
    );
    assert!(router.pricing_for("new-model").is_some());
    // Stale incoming entries don't clobber.
    let mut stale = PricingTable::new();
    stale.insert("gpt-4o", stamped(9.9, 1));
    router.merge_pricing(&stale);
    assert_eq!(
        router
            .pricing_for("gpt-4o")
            .expect("still fresh")
            .input_per_1m,
        0.0
    );
}

// --- JSON refresh (feature "json") -------------------------------------------------

#[cfg(feature = "json")]
mod json_tests {
    use super::*;
    use model_router::{PricingTable, RouterError};

    #[test]
    fn test_json_roundtrip() {
        let mut table = PricingTable::new();
        table.insert("gpt-4o", p(2.5).with_context_window(128_000));
        table.insert("glm-4.6", stamped(0.5, 1_757_289_600));

        let json = table.to_json().expect("serialize");
        let back = PricingTable::from_json(&json).expect("deserialize");
        assert_eq!(table, back);
    }

    #[test]
    fn test_json_missing_updated_at_defaults_to_none() {
        // Snapshots produced before 0.1.1 (no updated_at_unix field) load
        // with the stamp defaulting to None.
        let json = r#"{
            "gpt-4o": {
                "input_per_1m": 2.5,
                "output_per_1m": 10.0,
                "context_window": 128000,
                "max_output_tokens": 16384,
                "quality_tier": 4
            }
        }"#;
        let table = PricingTable::from_json(json).expect("legacy JSON loads");
        let entry = table.get("gpt-4o").expect("entry");
        assert_eq!(entry.updated_at_unix(), None);
        assert_eq!(entry.input_per_1m, 2.5);
        assert_eq!(entry.context_window, 128_000);
    }

    #[test]
    fn test_json_invalid_input_is_error() {
        let err = PricingTable::from_json("not json at all").unwrap_err();
        assert!(matches!(err, RouterError::Json(_)), "unexpected: {err}");

        // Valid JSON, invalid entry shape (missing required fields).
        let err = PricingTable::from_json(r#"{"gpt-4o": {}}"#).unwrap_err();
        assert!(matches!(err, RouterError::Json(_)), "unexpected: {err}");
    }

    #[test]
    fn test_json_shape_is_plain_model_map() {
        let mut table = PricingTable::new();
        table.insert("m", stamped(1.0, 100));
        let json = table.to_json().expect("serialize");
        // The wire format is a plain { model: pricing } object, hostable as-is.
        assert!(json.starts_with('{') && json.ends_with('}'), "{json}");
        assert!(json.contains("\"m\""), "{json}");
        assert!(json.contains("\"updated_at_unix\":100"), "{json}");
    }

    #[test]
    fn test_refresh_workflow_end_to_end() {
        // defaults → fetch fresh JSON (simulated) → merge → route.
        let fresh_json = r#"{
            "gpt-4o": {
                "input_per_1m": 1.25,
                "output_per_1m": 5.0,
                "context_window": 128000,
                "max_output_tokens": 16384,
                "quality_tier": 4,
                "updated_at_unix": 1757289600
            }
        }"#;
        let fresh = PricingTable::from_json(fresh_json).expect("fetch+parse");

        let mut router = Router::new("claude-sonnet-4-20250514");
        router.merge_pricing(&fresh);

        let gpt4o = router.pricing_for("gpt-4o").expect("refreshed entry");
        assert_eq!(gpt4o.input_per_1m, 1.25);
        assert_eq!(gpt4o.updated_at_unix(), Some(1_757_289_600));
        // Built-ins that the snapshot didn't mention are intact.
        assert!(router.pricing_for("glm-4.6").is_some());
    }
}
