# PATTERN_ADOPTION: model-router

Cost-aware LLM routing core (Rust library, not Node/TS): per-model
pricing tables, USD budget tracking, task-class routing with fallback
chains. Routing logic stays declarative; only canon catalog IDs with a
mechanism match in code are claimed.
Declared 2026-11-20 against pattern-standards references/catalog.md;
statuses and evidence follow references/adoption-format.md.

| Pattern | Status | Evidence |
|---|---|---|
| T13 | adopted | src/lib.rs:22-30 (the crate is deliberately free of HTTP clients, SDKs, and async runtimes), src/routing.rs:273-289 (route is a deterministic pure function from config to decision) |
| P119 | adopted | src/cost.rs:128-147 (record checks the budget before mutating; an over-budget record is rejected and the tracker is left unmodified), src/cost.rs:212-217 (over-budget state query), src/error.rs:13-19 (typed BudgetExceeded fault) |
