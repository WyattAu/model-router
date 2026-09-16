#![no_main]

use libfuzzer_sys::fuzz_target;
use model_router::{Router, RoutingRule, TaskClass, TaskComplexity};

/// Split `data` into up to `n` length-prefixed records (u16 LE length +
/// payload). Missing records come back empty; trailing bytes are ignored.
fn split_records(mut data: &[u8], n: usize) -> Vec<&[u8]> {
    let mut parts = Vec::with_capacity(n);
    while parts.len() < n && data.len() >= 2 {
        let len = u16::from_le_bytes([data[0], data[1]]) as usize;
        let rest = &data[2..];
        let take = len.min(rest.len());
        parts.push(&rest[..take]);
        data = &rest[take..];
    }
    while parts.len() < n {
        parts.push(b"");
    }
    parts
}

fuzz_target!(|data: &[u8]| {
    let data = &data[..data.len().min(2048)];
    let parts = split_records(data, 3);
    let default_model = String::from_utf8_lossy(parts[0]);
    let rule_model = String::from_utf8_lossy(parts[1]);
    let fallback_model = String::from_utf8_lossy(parts[2]);

    // Config assembly: arbitrary model names and fallback chains (including
    // self-referential chains) must build without panicking.
    let mut router = Router::new(default_model.as_ref());
    for class in [TaskClass::Fast, TaskClass::Balanced, TaskClass::Power] {
        let rule = RoutingRule::new(class, rule_model.as_ref())
            .with_fallback(RoutingRule::new(class, fallback_model.as_ref()));
        router = router.with_rule(rule);
    }

    // Phase-name mapping: arbitrary strings map to a class (total), and
    // routing/cost selection over the assembled config must stay total.
    for phase in [parts[0], parts[1], parts[2], b""] {
        let class = TaskClass::for_phase_name(&String::from_utf8_lossy(phase));
        let decision = router.route(class);
        let _ = decision.candidates().count();
    }
    let _ = router.select_by_complexity(TaskComplexity::Complex);
});
