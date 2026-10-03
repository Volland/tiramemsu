## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Implement Host and Executor against a chosen synchronous WASM SQLite runtime; document the runtime choice before implementation. Run in a worker to keep synchronous database work off the main UI thread. Declare functions, virtual tables, FTS5, statistics, and reader support from actual runtime behavior. Support the core tier first and enable queries only if required registration capabilities exist. Persistence and journal strategy must satisfy the existing format contract; unsupported WAL/storage combinations must fail explicitly, not quietly downgrade. Reuse tm-core and the format migration mechanism. Tests must open exported files across native and WASM hosts.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

- **Runtime.** `rusqlite` 0.40 on `wasm32-unknown-unknown` links `sqlite-wasm-rs` 0.5 (SQLite 3.53.0 compiled to WASM, VFSes in Rust), so `tm-wasm` reuses `tm-rusqlite`'s `RusqliteExec` and wraps it as `WasmExec`; OPFS is the sync-access-handle pool VFS of `sqlite-wasm-vfs` 0.2, installed asynchronously once in a dedicated worker (`install_opfs`), after which every call is synchronous. The crate also builds natively (`Storage::File`), which is how journal policy, crash recovery and interchange are tested beside `tm-rusqlite`.
- **Capabilities, observed.** Probed at host construction by running each feature: in WebAssembly (Node.js and headless Chrome) `functions`, `vtab` and `fts5` work, `stat4` is not compiled in, and `reader_pool` is never declared (`THREADSAFE=0`, no shared memory). The whole query engine runs in the browser; `limit_capabilities` gives the core tier, where the facade refuses the engine with `MissingCapability`.
- **Journal.** Both browser VFSes answer `PRAGMA journal_mode = WAL` with `delete`. The host checks the reported mode at open: `Journal::Wal` fails with `MissingCapability` naming what the runtime kept; `Journal::Rollback` is the explicit opt-in. The engine's own WAL switch is checked (Wal) or left out (Rollback), never silently changed. The file format is unchanged, so no migration is needed.
- **Interchange and recovery.** Files move with `export_file`/`import_file`; tests compare every engine row and counter across hosts in both directions, natively and through `scripts/wasm-interop.sh` (native -> WASM under Node.js -> native). A child process aborted mid-transaction leaves a hot journal that the next open rolls back. The facade now closes readers before the writer, so a closed native `Db` leaves no `-wal` and its main file alone is complete.
- **Binding.** `tiramemsu-wasm` serves the JSON bridge; budgets, bulk import sessions and `cancel` are refused with `Unsupported` because `std::time::Instant` is unavailable on the target and a worker runs one call at a time. `WasmClock` supplies `Date.now()`. SPARQL in WASM needs `--cfg getrandom_backend="wasm_js"` (set in `.cargo/config.toml`).
- **Gaps.** Killing a worker during an OPFS write is simulated by capturing storage during an open transaction, not by terminating a browser worker; persistence across page reloads is not exercised; no npm package is published; the release wasm is about 10.7 MB before `wasm-opt`.
