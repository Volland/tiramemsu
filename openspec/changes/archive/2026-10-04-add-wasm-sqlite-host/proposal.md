# WASM SQLite host

## Status

Implemented. New crate `tm-wasm` (`WasmHost`, `WasmExec`, `Storage::{Memory, Opfs, File}`, `Journal::{Wal, Rollback}`, `WasmClock`, `probe_capabilities`, `runtime_info`, `install_opfs`, `OpfsOptions`, `WasmHost::{limit_capabilities, export_file, import_file}`) and the wasm-bindgen binding `tiramemsu-wasm` (`bindings/wasm`: JS class `Database` with `open`, `call`, `capabilities`, `exportFile`, `close`, and `importFile`, `runtimeInfo`); the JSON bridge gains `Database::open_options` and `Database::from_db`.

## Why

A second SQLite host can make embedded memory usable in a browser and test the actual portability of the executor abstraction.

## What Changes

Add an opt-in WASM SQLite host with explicit capability reporting, durable storage configuration, and shared-format tests.

## Capabilities

### New Capabilities

- `wasm-sqlite-host`: wasm sqlite host.

### Modified Capabilities

None in this draft. Implementation must add delta modifications if an existing public requirement must change.

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No DuckDB, D1, or general backend abstraction.
- No automatic browser UI or network synchronization.
- No promise of query-language support when host capabilities are missing.
