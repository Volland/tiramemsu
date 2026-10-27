## Status

Closed without landing (2026-10-27): phase A was built and the gate was not met (see "Gate result" and "Closing" in `design.md`). Its requirements were not added to the specs. The unchecked tasks were not done.

## 1. Implementation

- [x] 1.1 (partial: `tm_core::sqlstore::SqlBackend`, without batch variants or `tm-wasm`) `ReadStore`/`WriteStore` on `tm-rusqlite` and `tm-wasm` with the existing statements; batch variants — `@tg: implements:: [[openspec:storage-trait#SQLite implements the storage trait]]`.
- [ ] 1.2 Facade writes through `tm_store::Engine`; `tm_core` engine reduced to re-exports.
- [ ] 1.3 `OpenOptions::runner = Native` on SQLite.

## 2. Tests and gate

- [x] 2.1 Whole workspace test suite on SQLite; oracle test trait-on-SQLite against the SQL engine (`tm-store/tests/sqlite_backend.rs`).
- [x] 2.2 (measured, not met) Benchmarks before and after — `@tg: implements:: [[openspec:storage-trait#No regression on SQLite]]`.

## 3. Documentation and checks

- [ ] 3.1 lat.md `architecture.md#Executor`, `#Storage Backends`; `tg check`, `tg ontology check`, `tg trace storage-trait --gaps`, `openspec validate move-sqlite-onto-store`.
