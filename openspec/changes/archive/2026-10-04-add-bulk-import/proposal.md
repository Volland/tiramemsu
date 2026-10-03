# Bulk import with deferred statistics

## Status

Implemented. Rust: `Db::bulk_import`, `Db::bulk_import_shared`, `BulkImport::{chunk, chunk_with, progress, finish, cancel}` (drop = cancel), `ImportProgress`, `ImportSummary`, `Db::statistics_due`, `Db::import_active`, and the error `ImportInProgress`. JSON bridge: `importBegin`, `importChunk`, `importProgress`, `importFinish`, `importCancel`, and `importActive`/`statisticsDue` in `info`; Node `Database.bulkImport()` → `BulkImport` and Python `Database.bulk_import()` → `BulkImport` (context manager) with `ImportProgress`/`ImportSummary`.

## Why

The current 1,000-row chunk trigger performs full ANALYZE after every chunk. Large imports need explicit transaction boundaries and one final planner refresh.

## What Changes

Add an opt-in import session, explicit finalization, progress reports, and statistics timing while preserving ordinary transaction behavior.

## Capabilities

### New Capabilities

- `bulk-import`: bulk import with deferred statistics.

### Modified Capabilities

None in this draft. Implementation must add delta modifications if an existing public requirement must change.

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No storage schema or history-index redesign.
- No new RDF format parser.
- No automatic rollback of previously committed chunks.
