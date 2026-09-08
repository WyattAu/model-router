# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

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
