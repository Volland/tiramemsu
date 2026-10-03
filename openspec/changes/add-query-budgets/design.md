## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Keep SQLite busy_timeout separate from pool acquisition and execution deadlines. Check cancellation during native frontier expansion and use host execution interruption for SQL. Fail atomically rather than returning an unmarked prefix. Limits apply across provenance sibling queries and frontend sub-operations, not anew for each statement. Defaults preserve existing behavior; bounded presets can be used by adapters.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

The open decisions were settled in implementation. The budget is the facade struct `QueryBudget` (`timeout`, `cancel: CancelToken`, `reader_timeout`, `max_rows`, `max_bytes`), applied through `View::with_budget`, `Db::transact_budgeted`, `Db::cypher_write_budgeted` and `QueryBudget::run`; `OpenOptions::reader_timeout` is the database-wide reader wait (default unbounded). The running operation's meter lives in `tm_core::budget` on the calling thread, so nested statements share it; hosts receive the stop conditions through the default-no-op `Executor::set_interrupt`, which the `rusqlite` host maps to SQLite's progress handler. Errors are `Cancelled`, `DeadlineExceeded`, `PoolTimeout` and `ResultLimitExceeded`, with the same JSON codes. No schema or file-format change was needed.
