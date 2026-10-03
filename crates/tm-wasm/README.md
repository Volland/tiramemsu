# tm-wasm

The WebAssembly SQLite host for tiramemsu: SQLite compiled to `wasm32-unknown-unknown` behind the `tm-core` `Host` and `Executor` traits, with in-memory or OPFS storage.

Tiramemsu is a layered, never-forget memory for agents, stored in one SQLite file. `tm-core` reaches SQLite only through its own traits; [`tm-rusqlite`](https://crates.io/crates/tm-rusqlite) implements them natively, and this crate implements them in the browser. The file format is the same, so a database written in a Web Worker opens natively and the other way round.

```text
 tiramemsu      facade: Db, View, Tx
    |
 tm-core        Store, Tx, views, Executor / Host traits
    |
 tm-wasm        WasmHost, WasmExec   <- this crate
    |
 tm-rusqlite    RusqliteExec (statements, registration, interrupts)
    |
 rusqlite --> sqlite-wasm-rs (SQLite compiled to WASM, memvfs, opfs-sahpool)
```

For JavaScript, use the [`tiramemsu-wasm`](https://github.com/Volland/tiramemsu/tree/main/bindings/wasm) binding, which serves the JSON bridge from a Web Worker on top of this host.

## Runtime choice

`rusqlite` 0.40 builds for `wasm32-unknown-unknown` by linking [`sqlite-wasm-rs`](https://crates.io/crates/sqlite-wasm-rs) 0.5, which compiles the SQLite amalgamation to WebAssembly with VFSes written in Rust. The host therefore reuses the `rusqlite` executor of `tm-rusqlite` unchanged (prepared statements, savepoints, scalar and aggregate functions, eponymous virtual tables, `rarray`, the progress-handler interrupt) and adds what differs: storage selection, the journal policy and the capability probe. No JavaScript SQLite build is involved and every call is synchronous, which is why the database belongs in a worker. OPFS storage uses the sync-access-handle pool VFS of [`sqlite-wasm-vfs`](https://crates.io/crates/sqlite-wasm-vfs) 0.2.

## Capabilities, as observed

The host probes the running SQLite when it is built (`probe_capabilities`, `runtime_info`) by doing each thing on a scratch connection. In WebAssembly (SQLite 3.53.0 from `sqlite-wasm-rs` 0.5.5, under Node.js and in headless Chrome):

| Capability | WASM runtime | Why |
|---|---|---|
| `functions` | yes | a registered scalar function runs |
| `vtab` | yes | the `rarray` eponymous table answers; `tm_path`, `tm_text` and `tm_lftj` register |
| `fts5` | yes | compiled with `ENABLE_FTS5`; text recall works |
| `stat4` | no | not compiled in; `ANALYZE` still fills `sqlite_stat1` |
| `reader_pool` | no | `THREADSAFE=0` and no shared memory in either VFS, so no WAL readers |

So the whole query engine (SPARQL, Cypher, paths, LFTJ, text recall) runs in the browser. `WasmHost::limit_capabilities` narrows the declared set, for example to the `tm-core` tier, and then opening a query engine fails with `MissingCapability`.

## Storage and journal

| Storage | Where | `Journal::Wal` | `Journal::Rollback` |
|---|---|---|---|
| `Storage::Memory` (`memvfs`) | any context, `wasm32-unknown-unknown` | `MissingCapability`: the runtime keeps `delete` | volatile, rollback journal |
| `Storage::Opfs` (`opfs-sahpool`) | dedicated Web Worker, after `install_opfs` | `MissingCapability`: the runtime keeps `delete` | durable, rollback journal |
| `Storage::File` | native targets (tests) | WAL | rollback journal |

The native format contract is WAL. Neither WASM VFS has shared memory, so SQLite answers `PRAGMA journal_mode = WAL` with `delete`. The host checks the mode the runtime actually reports when it opens a connection and fails rather than continuing in another mode: `Journal::Rollback` is the explicit opt-in to a rollback journal. The database file is the same either way; a hot journal is rolled back on the next open, and a native host switches the file back to WAL when it opens it.

## Usage

```rust
use std::sync::Arc;
use tiramemsu::{Db, OpenOptions, TxOptions, Valid, Value};
use tm_wasm::{Journal, Storage, WasmClock, WasmHost};

# let dir = tempfile::tempdir()?;
# let path = dir.path().join("memory.db");
// in a worker: Storage::Memory, or Storage::Opfs after `tm_wasm::install_opfs(..).await`
let host = WasmHost::new(Storage::default_for_target(), Journal::Rollback);
let opts = OpenOptions {
    clock: Arc::new(WasmClock), // Date.now(): SystemTime is unavailable on wasm32
    ..OpenOptions::default()
};
let db = Db::open_with_host(host.clone(), &path, opts)?;
db.transact(TxOptions::default(), |tx| {
    tx.assert(Value::iri("urn:x:alice"), Value::iri("urn:x:knows"), Value::iri("urn:x:bob"), Valid::ALWAYS)
        .map(|_| ())
})?;
drop(db);
// the committed bytes, to download or to open with the native host
let bytes = host.export_file(path.to_str().unwrap())?;
assert!(bytes.starts_with(b"SQLite format 3\0"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

A natively written file goes in with `WasmHost::import_file`; the memory and OPFS VFSes import only the main file, and a closed native `Db` leaves no `-wal` behind.

## Limits

- **No time source in std.** `std::time::Instant` and `SystemTime` panic on `wasm32-unknown-unknown`: pass `WasmClock` as the clock, and do not use query budgets (`QueryBudget`) or bulk import sessions there. The JavaScript binding refuses them.
- **SPARQL needs a cfg.** `oxrdf` draws blank-node ids through `getrandom` 0.3; a crate building the SPARQL front end for the web sets `--cfg getrandom_backend="wasm_js"` (this workspace does so in `.cargo/config.toml`).
- **One connection.** The writer serves every read; there is no reader pool.

## Tests

`cargo test -p tm-wasm` runs the host natively on files. In WebAssembly under Node.js (install the `wasm-bindgen-cli` version from `Cargo.lock`):

```text
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked
cargo test -p tm-wasm --target wasm32-unknown-unknown --tests
scripts/wasm-interop.sh   # native file -> WASM -> native, row for row
CHROMEDRIVER=... cargo test -p tm-wasm --target wasm32-unknown-unknown --features opfs-tests --test opfs
```

Part of [tiramemsu](https://github.com/Volland/tiramemsu). Licensed under MIT or Apache-2.0.
