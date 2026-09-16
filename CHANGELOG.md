# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

## [0.2.0] - 2026-09-16

### Changed
- **BREAKING**: `ModelPricing` gained a private `updated_at_unix` field — the
  struct is no longer constructible with a struct expression
  (`constructible_struct_adds_private_field`).
- **BREAKING**: `RouterError` gained a `Json` variant (exhaustive enum,
  `enum_variant_added`).

Both breaking changes landed across 0.1.1–0.1.4, which semver-checks
correctly flags for a 0.x crate (breaking changes require a minor bump
while pre-1.0). This release cuts 0.2.0 so the crate can be consumed
against the v0.1.0 baseline. First release with a committed Cargo.lock
(the CI `check --locked` gate requires it).

## [0.1.4] - 2026-09-12

### Added
- Config-knob behavior matrix (`tests/config_matrix.rs`, committed
  2026-09-12): all 11 knobs behavior-proven — default model, per-class
  rules, fallback chains, per-model pricing, pricing merge freshness,
  quality-tier gating, context/output limits, budget enforcement,
  provider defaults, and pricing accessors (`updated_at` builders covered
  in `tests/pricing_freshness.rs`). Dead-knob sweep found zero dead knobs.

### Fixed
- Crate docs linked `PricingTable::from_json` / `to_json`
  unconditionally although they exist only with the `json` feature;
  reworded so `cargo doc --no-deps` is warning-free with default features.

## [0.1.3] - 2026-09-12

### Added
- `tests/config_matrix.rs` (10 tests): behavior-observable coverage for
  every public config/builder knob — `set_default_model`, `with_rule`,
  `with_fallback` (including replace-not-append semantics),
  `with_pricing`/`add_pricing`, `merge_pricing` freshness, `with_quality_tier`
  gating `select_by_complexity`, `with_context_window`,
  `with_max_output_tokens`, budget enforcement, and provider-specific
  `with_default_rules`. Dead-knob sweep found zero dead knobs.

## [0.1.2] - 2026-09-12

### Added

- Router-seam wire test suite (`tests/wire_routing.rs`, 7 tests) against
  mock OpenAI-compatible providers (wiremock on loopback): request shape
  (routed model + messages over the wire), usage-based cost accounting,
  failover from a failing primary (500) to a fallback (200) walking
  `candidates()` in order, all-candidates-failing surfacing, slow-provider
  abandonment via client timeouts, cost-aware `select_by_complexity`
  quality-floor selection, budget cap enforcement (`BudgetExceeded`
  rejected before state mutation), and pricing refresh via JSON updating
  live cost estimates. (The optional `genai` companion hardcodes provider
  endpoints, so the wire boundary is exercised at the router seam.)

### CI

- New `integration` job running the wire-routing suite.

## [0.1.1] - 2026-09-08

### Added
- Price-table freshness (`updated_at_unix`, Unix seconds) — addresses the
  audit finding that static price tables go stale:
  - `ModelPricing::updated_at_unix()` getter plus
    `with_updated_at_unix(u64)` / `with_updated_at_now()` builders. The
    built-in defaults are unstamped (`None` = freshness unknown) and the
    field deserializes as `None` when absent, so pre-0.1.1 JSON still loads.
  - `PricingTable` — refreshable pricing set with
    `PricingTable::merge(&mut self, other)` (upsert entries that are missing
    or stamped strictly newer; stale and unstamped incoming entries never
    clobber) and `Router::merge_pricing(&PricingTable)`.
  - `PricingTable::from_json(&str)` / `PricingTable::to_json()` behind a new
    `json` feature (implies `serde`, adds optional `serde_json`), so tables
    can be refreshed at runtime from a hosted JSON file without a crate
    release. JSON failures surface as `RouterError::Json`.
- Refresh workflow documentation in the README (host a JSON table, fetch,
  merge).

## [0.1.0] - 2026-09-04

### Added
- Cost-aware LLM routing core — per-model pricing tables and USD budget tracking.
- Published to crates.io (2026-09-04).
