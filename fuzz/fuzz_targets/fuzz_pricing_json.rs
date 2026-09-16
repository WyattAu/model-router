#![no_main]

use libfuzzer_sys::fuzz_target;
use model_router::{CostTracker, PricingTable};

fuzz_target!(|data: &[u8]| {
    // Bound input so JSON parsing stays fast.
    let data = &data[..data.len().min(8192)];
    let json = String::from_utf8_lossy(data);

    // Pricing-table load path: adversarial JSON (wrong types, negative and
    // non-finite rates, unknown fields, huge nesting) must load-or-Err,
    // never panic.
    if let Ok(mut table) = PricingTable::from_json(&json) {
        // Merging a loaded table into the defaults must not panic, and a
        // re-serialization round-trip must be Ok (Err is acceptable only if
        // the crate documents unserializable tables).
        let defaults = PricingTable::with_defaults();
        table.merge(&defaults);
        let _ = table.to_json();

        // Lookups and cost math over loaded rates: NaN/inf pricing must not
        // panic downstream consumers.
        if let Some((model, pricing)) = table.iter().next() {
            let model = model.to_string();
            let tracker = CostTracker::new(Some(1_000.0));
            let _ = tracker.record(&model, usize::MAX, usize::MAX, pricing);
            let _ = tracker.record(&model, 0, 1, pricing);
            let _ = tracker.total_usd();
            let _ = tracker.report();
        }
    }

    // Default-table lookups over arbitrary model-name keys must stay
    // total (Option-returning), never panic.
    let defaults = PricingTable::with_defaults();
    let _ = defaults.to_json();
});
