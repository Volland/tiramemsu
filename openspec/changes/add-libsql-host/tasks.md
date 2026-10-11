## Status

Deferred (2026-10-11): not the vector path; an optional native host that may later offer an approximate tier. Nothing implemented. Only groups 1 to 5 and 8 apply if the deferral is lifted; groups 6 and 7 (default switch, phase-3 decision) are withdrawn and kept for reference.

## 1. Spike and pinning (phase 1)

- [ ] 1.1 Build a throwaway binary on `libsql-rusqlite` (`bundled`, `functions`, `vtab`, `array`, `hooks`) on Linux, macOS and Windows; record compile time, binary size, `sqlite_version()`, `libsql_version()` and `PRAGMA compile_options`.
- [ ] 1.2 Decide the pin: 0.10.0-pre.x (SQLite 3.47.0) or 0.9.30 (SQLite 3.45.1) with the table-scoped `ANALYZE` fallback; record it in `design.md`.
- [ ] 1.3 Confirm the duplicate-symbol conflict of linking `libsqlite3-sys` (bundled) and `libsql-ffi` into one binary on all three OSes; record the linker output.
- [ ] 1.4 Build a real libSQL vector index in a file and open it with stock SQLite 3.53: confirm which statements fail (`ANALYZE`, `integrity_check`, `VACUUM`, `REINDEX`, writes to the indexed table) and which succeed; check whether `libsql_vector_meta_shadow` is created eagerly on a fresh file.

## 2. Core capability flag

- [ ] 2.1 Add `Capabilities::vector` in `tm-core` (doc comment, default `false`); update every struct literal in the workspace.
- [ ] 2.2 `RusqliteHost` and `WasmHost` declare `vector = false`; `tm-wasm`'s probe reports it in `RuntimeInfo`.
- [ ] 2.3 Extend the `tm-core` dependency check in `ci.yml` to reject `libsql`, `libsql-ffi`, `libsql-sys` and `libsql-rusqlite`.

## 3. `tm-libsql` crate

- [ ] 3.1 Create `crates/tm-libsql` (`LibsqlHost`, `LibsqlExec`, `map_err`, `register`, `compile_options`, `runtime_info`, `limit_capabilities`), README, crate docs, workspace entry.
- [ ] 3.2 Port `lib.rs`, `register.rs`, `error.rs` from `tm-rusqlite`; adapt `progress_handler` (returns `()`), keeping the rule that a borrowed handle installs no interrupt.
- [ ] 3.3 Port `table_fn.rs` (eponymous modules via `eponymous_only_module()`, `Values` in `filter`, `from_handle` borrowed executor, error slot); keep `unsafe` confined to this module with `deny(unsafe_code)` elsewhere.
- [ ] 3.4 Implement the capability probe (D3), including the `vector` probe (`F32_BLOB`, `libsql_vector_idx`, `vector_top_k`, `vector_distance_cos`).
- [ ] 3.5 Set `PRAGMA foreign_keys` explicitly to the stock default on every connection.
- [ ] 3.6 Statistics guard: after the open-time upkeep on a non-empty file, verify `sqlite_stat1` rows for `triple`; otherwise run table-scoped `ANALYZE` over the engine tables.

## 4. Facade host selection

- [ ] 4.1 Add facade features `host-rusqlite` (default in phase 1) and `host-libsql`; `NativeHost` alias; `compile_error!` when both are enabled; `Db::open` uses the selected host.
- [ ] 4.2 Thread the host features through `tiramemsu-json`, `tiramemsu-mcp`, `bindings/python`, `bindings/node` without changing their defaults.
- [ ] 4.3 Add a host axis to `scripts/feature-matrix.sh` (`core`, `exec`, `sparql`, `cypher`, `default` × `rusqlite`, `libsql`).

## 5. Tests (phase 1)

- [ ] 5.1 Host conformance suite parameterised by host: required operations, savepoints, stable snapshots, failed-transaction rollback, registration on readers, borrowed-connection reads, typed-error slot, interrupt and pool return.
- [ ] 5.2 Acceptance tests for every scenario in `specs/libsql-host`, `specs/sql-executor` and `specs/wasm-sqlite-host`.
- [ ] 5.3 CI job `hosts`: matrix `host × os` running the workspace suite, SPARQL and Cypher differential suites, TCK counts (must be identical per host), feature matrix, Python and Node smoke tests.
- [ ] 5.4 CI job `host-interchange` (two builds, two processes): write the format-4 fixture, compare every engine row and counter, extend, hand back; repeat with a killed writer and a hot WAL.
- [ ] 5.5 Stock-host-with-vector-index test: create the file with the libSQL build (raw SQL; no engine code creates such tables until an approximate tier exists), open it with the rusqlite build, run the core suite fixture against it, assert the `vec_` tables are unchanged and libSQL vector calls fail with `MissingCapability("vector")` while exact vector recall works.
- [ ] 5.6 Run the release benchmarks (churn, point/2-hop, path, supersede, bulk import, text recall) and the plan-quality fixture on both hosts; record results and plan differences in `crates/tiramemsu/benches/README.md`.

## 6. Phase 2: default switch (withdrawn 2026-10-11)

- [ ] 6.1 Confirm the phase-1 exit criteria and record the evidence in `design.md`.
- [ ] 6.2 Make `host-libsql` the default of the facade, JSON bridge, MCP server, Python wheels and Node addon; keep `host-rusqlite` documented.
- [ ] 6.3 Update the wheel and addon build workflows; smoke-test published artifacts on every platform.
- [ ] 6.4 Release notes: SQLite version change, `vector` capability, how to opt back into stock SQLite, symbol conflict with other bundled-SQLite crates.

## 7. Phase 3: decision (withdrawn 2026-10-11)

- [ ] 7.1 Decide whether `tm-rusqlite` stays a supported native fallback or becomes the WASM base only; record inputs and the decision in `lat.md/architecture.md#Executor`.
- [ ] 7.2 Adjust the CI matrix to the decision.

## 8. Documentation and validation

- [ ] 8.1 Update `lat.md/architecture.md` (Crates, Cargo Features, Executor, WebAssembly Host) and the facade README; run `lat check`.
- [ ] 8.2 Validate with `openspec validate add-libsql-host --strict`; archive only after implementation.
