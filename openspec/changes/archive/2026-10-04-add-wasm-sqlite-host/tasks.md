## Status

Implemented. New crate `tm-wasm` (`WasmHost`, `WasmExec`, `Storage::{Memory, Opfs, File}`, `Journal::{Wal, Rollback}`, `WasmClock`, `probe_capabilities`, `runtime_info`, `install_opfs`, `OpfsOptions`, `WasmHost::{limit_capabilities, export_file, import_file}`) and the wasm-bindgen binding `tiramemsu-wasm` (`bindings/wasm`: JS class `Database` with `open`, `call`, `capabilities`, `exportFile`, `close`, and `importFile`, `runtimeInfo`); the JSON bridge gains `Database::open_options` and `Database::from_db`.

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement wasm sqlite host behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec (archiving is left to a later step).
