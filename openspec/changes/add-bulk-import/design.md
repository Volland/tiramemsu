## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Use the existing transaction engine for every chunk. Completed chunks remain committed if a later chunk fails; report their transaction ids and counts. Hold an exclusive import-session lease for writes, but allow committed readers. Suppress automatic analysis during the session and run full analysis plus reader-statistics refresh on explicit finish. Cancellation or dropping a session releases the lease and marks statistics due for later upkeep; dropping must not run expensive analysis. A failed final analysis reports committed data separately from maintenance failure. This is not an atomic multi-chunk transaction.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

- **API:** `Db::bulk_import()` returns `BulkImport<'_>`, and `Db::bulk_import_shared(&Arc<Db>)` returns `BulkImport<'static>` so bindings can keep a session between calls. Chunks are `chunk(f)` and `chunk_with(opts, budget, f)`, the closure form of `Db::transact`. `finish()` returns `ImportSummary { progress, analyzed, maintenance_error, statistics_due }` and never fails; `cancel()` returns `ImportProgress`, and drop is equivalent.
- **Lease:** an atomic session id on `Db`, compared under the writer mutex in every write. A write from outside the session, or a second session, fails fast with the new `Error::ImportInProgress` instead of blocking: blocking would deadlock a single-threaded caller (Node, Python) holding a session. Speculation (`with`) and `optimize` are not writes and stay allowed. Readers are unaffected.
- **Deferred statistics:** `tm_core::storage::stats::Stats` gained a deferred flag and a due flag. The facade sets deferral per transaction under the writer lock, on for session chunks and off for every other write, so dropping a session needs no writer lock and runs no analysis. Deferred commits mark statistics due, and the first ordinary commit afterwards runs the usual `ANALYZE` + `PRAGMA optimize`, as does `Db::optimize`. `Db::statistics_due()` exposes the flag. The flag lives in memory only; after a reopen the existing open-time check (no STAT4 samples) covers a fresh file.
- **Final analysis:** `finish` runs `Store::optimize` (full `ANALYZE` plus the schema-cookie bump that makes pooled readers reload statistics), then releases the lease whatever happened. A failure, such as `SQLITE_BUSY` from another process holding the write lock (the acceptance test), is reported as `maintenance_error` beside the committed chunks.
- **Progress:** committed chunks, rejected chunks, asserted/existing/retracted rows, every committed chunk's `TxId`, chunk time and maintenance time. A dry-run chunk is counted neither as committed nor as rejected.
- **No format change:** no schema, trigger or history-index change; default behaviour without a session is unchanged.
- **Bindings:** the bridge keeps sessions in a map by id (`Arc<Mutex<Option<BulkImport<'static>>>>`, so one session's chunks serialise), and `Database` now holds `Arc<Db>`. A session that a binding never ends holds the lease until the database is closed; the wrappers document `try/finally` (Node) and the context manager (Python).
