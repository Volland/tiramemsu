## Status

Implemented. Rust: `QueryBudget` (`timeout`, `cancel`, `reader_timeout`, `max_rows`, `max_bytes`), `CancelToken`, `View::with_budget`, `Db::transact_budgeted`, `Db::cypher_write_budgeted`, `QueryBudget::run`, `OpenOptions::reader_timeout`, `Executor::set_interrupt`, and the errors `Cancelled`, `DeadlineExceeded`, `PoolTimeout`, `ResultLimitExceeded`. JSON bridge: per-call `budget` (`timeoutMs`, `readerTimeoutMs`, `maxRows`, `maxBytes`, `cancelKey`), the `cancel` operation and the open option `readerTimeoutMs`; Node `withBudget`/`cancel` and Python `QueryBudget`/`with_budget`/`cancel`. Archiving is still pending.

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement query budgets and bounded reader acquisition behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec, and archive only after implementation is complete.
