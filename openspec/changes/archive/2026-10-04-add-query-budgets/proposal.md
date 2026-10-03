# Query budgets and bounded reader acquisition

## Status

Implemented (not yet archived). The public names are `QueryBudget` with `View::with_budget`, `Db::transact_budgeted`, `Db::cypher_write_budgeted` and `QueryBudget::run`; `CancelToken`; `OpenOptions::reader_timeout`; `Executor::set_interrupt`; and the errors `Cancelled`, `DeadlineExceeded`, `PoolTimeout` and `ResultLimitExceeded`, with the same codes and a per-call `budget` plus a `cancel` operation on the JSON bridge, Node and Python.

## Why

A long query or exhausted reader pool can stall an agent indefinitely. Make waiting, execution, and result growth independently bounded.

## What Changes

Add per-query deadlines, cancellation, reader-acquisition timeouts, and result limits with typed errors shared by the Rust and JSON APIs.

## Capabilities

### New Capabilities

- `query-budgets`: query budgets and bounded reader acquisition.

### Modified Capabilities

None in this draft. Implementation must add delta modifications if an existing public requirement must change.

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No networking or MCP implementation.
- No change to temporal visibility or path semantics.
