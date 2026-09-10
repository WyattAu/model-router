# Threat Model — model-router

Reference: STRIDE. Scope: the crate's public API surface (`CostTracker`,
`PricingTable::from_json`/`merge`, `Router`, `ModelPricing`) as used by a
downstream service. Trust boundaries: (1) JSON pricing documents entering
`from_json` (the runtime refresh workflow), (2) usage records entering
`CostTracker::record`, (3) concurrent callers sharing one tracker,
(4) the dependency tree (serde_json).

The crate is deliberately pure — no HTTP, no async, no I/O. Its security
surface is **financial integrity** (budget enforcement) and the integrity of
the pricing-refresh path.

## Assets

| ID | Asset | Example |
|----|-------|---------|
| A1 | Budget invariant: tracked spend never exceeds the limit | A record slipping past the cap via a check-after-write race |
| A2 | Integrity of pricing data | A stale or attacker-flattened pricing table silently mispricing every call |
| A3 | Correctness of routing/fallback decisions | A corrupted rule chain routing `Power` tasks to free-tier models |

## STRIDE Analysis

| # | Threat | Category | Surface | Mitigation | Verifying test |
|---|--------|----------|---------|------------|----------------|
| T1 | Spend exceeding budget (check-then-act race or post-hoc write) | Elevation/Tampering | `CostTracker::record` | Budget is checked **before** mutating state (checked-then-added semantics); a record that would breach the cap is rejected outright; accumulation is monotonic | `prop_budget_is_never_overshot`, `prop_cost_accumulation_is_monotonic`, `test_cost_tracker_budget_enforcement` (`tests/properties.rs`, `tests/unit.rs`) |
| T2 | Malicious/malformed pricing JSON corrupting the table | Tampering | `PricingTable::from_json` | Typed serde deserialization — invalid input returns an error rather than a partially-mutated table; unknown shapes rejected | `test_json_invalid_input_is_error`, `test_json_shape_is_plain_model_map` (`tests/unit.rs`); `test_json_roundtrip` |
| T3 | Refresh regression (older pricing overwriting newer) | Tampering | `PricingTable::merge` | Newer-wins merge keyed on freshness stamps: undated entries never overwrite dated ones; stale entries are preserved against undated pushes | `test_merge_newer_entry_wins`, `test_merge_undated_never_overwrites`, `test_merge_stale_entry_preserved`, `test_merge_stamped_wins_over_undated_self` (`tests/pricing_freshness.rs`) |
| T4 | Unauthenticated pricing source | Spoofing | the refresh workflow feeding `from_json` | **Not mitigated** — the crate cannot authenticate where JSON came from; an attacker who controls the fetched document controls prices (and thus budget arithmetic). Documented: fetch over authenticated/TLS channels and verify the source | Code review; no signature field exists in the JSON schema |
| T5 | Float pathology (NaN/inf costs) poisoning accumulation | Tampering | `CostTracker::record` arithmetic | Costs are computed from token counts × rates; estimates and recorded deltas are cross-checked by property; NaN sources (0×∞ style) are not explicitly rejected — documented gap | `prop_record_delta_matches_estimate` (delta consistency); no explicit `is_finite` gate — residual |
| T6 | Routing decision manipulation via rule order | Elevation | `Router` construction | Fallback chain ordering is caller-declared data, deterministic and stable under merge; no ambient mutation path | `prop_fallback_ordering_stable` (`tests/properties.rs`), `test_router_merge_pricing` |
| T7 | Unbounded JSON size on refresh | DoS | `from_json` | **Not mitigated** — no document-size cap; serde_json allocates for the whole input. Documented residual: cap response size at the fetch layer | Code review |

## Repudiation

Partially supported: `CostReport` snapshots per-model breakdowns and totals
at query time, giving an auditable spend trail *per tracker instance* — but
records carry no actor identity, so "who triggered this spend" is not
attributable inside the crate.

## Out of Scope

- Fetching and authenticity of pricing documents (TLS, signature checks) —
  caller workflow.
- Provider-side billing truth: this crate *estimates* cost from token
  counts; reconciliation with invoices is external.
- Enforcement *reaction*: what the service does on `is_over_budget` (hard
  stop, degrade, alert) is caller policy; `record` only rejects the
  breaching record.

## Residual Risks

- **R1 (Medium, accepted):** No source authentication for pricing refreshes
  (T4). The integrity guarantees in T2/T3 are conditional on the JSON
  arriving from a trusted channel.
- **R2 (Low, accepted):** f64 accumulation is not transactional across
  threads beyond the internal synchronization: concurrent `record` calls
  serialize, but a Nan/inf record (T5) would corrupt totals permanently.
  An `is_finite` guard is cheap future hardening.
- **R3 (Low, accepted):** Budget rejection is per-record: a single huge
  record is refused, but many just-under-threshold records still land the
  tracker at exactly the cap — by design, but callers expecting headroom
  should budget below the true limit.
- **R4 (Low, accepted):** Dependency risk in serde_json; no in-repo
  `cargo audit` gate (org-level Dependabot only).
