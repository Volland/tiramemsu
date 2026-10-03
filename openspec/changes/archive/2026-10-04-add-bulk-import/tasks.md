## Status

Implemented. Rust: `Db::bulk_import`, `Db::bulk_import_shared`, `BulkImport::{chunk, chunk_with, progress, finish, cancel}` (drop = cancel), `ImportProgress`, `ImportSummary`, `Db::statistics_due`, `Db::import_active`, and the error `ImportInProgress`. JSON bridge: `importBegin`, `importChunk`, `importProgress`, `importFinish`, `importCancel`, and `importActive`/`statisticsDue` in `info`; Node `Database.bulkImport()` → `BulkImport` and Python `Database.bulk_import()` → `BulkImport` (context manager) with `ImportProgress`/`ImportSummary`. Archiving is still pending.

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement bulk import with deferred statistics behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec, and archive only after implementation is complete.
