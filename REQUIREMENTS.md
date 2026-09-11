# Requirements — model-router

Numbered, testable requirements. Every requirement maps to at least one named
test or doc-comment contract; security-relevant items cite THREAT-MODEL.md rows.

Scope: LLM model router — provider-agnostic routing, failover, and usage accounting

## Functional

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-MR-001 | Routing selects providers by declared policy order; failover advances on transport/auth failures only | MUST |
| REQ-MR-002 | Usage accounting totals tokens per request exactly once (no double counting across retries) | MUST |
| REQ-MR-003 | Provider credentials are injected, never constructed or logged by the router | MUST |

## Security

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-MR-100 | API keys are passed opaquely; error rendering excludes credential material | MUST |
| REQ-MR-101 | Hostile provider responses cannot bypass typed parsing (errors on malformed payloads) | MUST |

## Observability & API hygiene

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-MR-900 | All fallible public APIs return typed errors; production `unwrap`/`expect` is denied or explicitly justified with an invariant comment | MUST |
| REQ-MR-901 | Public items carry doc comments with runnable examples where practical | SHOULD |

Reviewed: 2026-09-11
