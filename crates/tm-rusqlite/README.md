# tm-rusqlite

The rusqlite host for tiramemsu: bundled SQLite behind the `tm-core` `Host` and `Executor` traits.

Tiramemsu is a layered, never-forget memory for agents, stored in one SQLite file. `tm-core` reaches SQLite only through its own traits and links no SQLite binding; this crate implements those traits on [`rusqlite`](https://crates.io/crates/rusqlite) with a bundled SQLite. It is the only host in the workspace.

```text
 tiramemsu      facade: Db, View, Tx        <- most applications stop here
    |
 tm-sparql / tm-cypher --> tm-ir --> tm-exec
    |
 tm-core        Store, Tx, views, Executor / Host traits
    |
 tm-rusqlite    RusqliteHost, RusqliteExec   <- this crate
    |
 rusqlite (bundled SQLite)
```

Most applications should use the [`tiramemsu`](https://crates.io/crates/tiramemsu) facade, which already uses this crate. Depend on `tm-rusqlite` directly when you drive `tm-core` yourself, need to register your own SQL functions on every connection, or want to wrap an existing `rusqlite::Connection`.

## Usage

```rust
use tm_core::{vocab::v, Host, Store, StoreOptions, TxOptions, Valid, Value};
use tm_rusqlite::RusqliteHost;

# let dir = tempfile::tempdir()?;
# let path = dir.path().join("memory.db");
let host = RusqliteHost::new();
assert!(host.capabilities().functions && host.capabilities().vtab);

let mut store = Store::open(&host, &path, StoreOptions::default())?;
store.transact(TxOptions::default(), |tx| {
    tx.assert(Value::iri(v("alice")), Value::iri(v("knows")), Value::iri(v("bob")), Valid::ALWAYS)?;
    Ok(())
})?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

To add your own SQL to every connection the host opens (writer and readers), pass a callback:

```rust
use std::sync::Arc;
use tm_rusqlite::RusqliteHost;

let host = RusqliteHost::new().with_register(Arc::new(|conn| {
    conn.create_scalar_function(
        "twice",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8 | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| Ok(ctx.get::<i64>(0)? * 2),
    )
}));
# let _ = host;
```

## Tour

- [`RusqliteHost`]: implements `tm_core::Host`. `open_writer` opens a read-write, creating connection; `open_reader` a read-only one. Both set the busy timeout, a 256-entry prepared-statement cache and the default registrations.
- [`RusqliteExec`]: implements `tm_core::Executor` on one connection. `RusqliteExec::from_connection` wraps a connection you opened yourself, declaring the capabilities you choose; `RusqliteExec::connection` gives the `rusqlite::Connection` back.
- [`RegisterFn`], [`RusqliteHost::with_register`]: the per-connection hook shown above.
- [`register::register_defaults`]: what every connection gets by default (the `rarray` table-valued function, used for chunked frontiers).
- [`map_err`]: turns a `rusqlite::Error` into `tm_core::Error::Sqlite` with the SQLite result code kept.
- [`compile_options`]: the linked SQLite's `PRAGMA compile_options`.

## Design notes

- **Capabilities.** [`RusqliteHost::new`] declares `reader_pool`, `functions` and `vtab` always, and `stat4` and `fts5` only if `PRAGMA compile_options` of the bundled SQLite lists `ENABLE_STAT4` / `ENABLE_FTS5`. The query crates (`tm-exec`) use the executor's `registry()` to install native functions, aggregates and table functions such as `tm_path`; they refuse a host without `functions` and `vtab`.
- **Registration.** `registry()` returns `Some` when `functions` or `vtab` is declared. It supports scalar and aggregate functions, plain table functions, and table functions that read back through the calling connection.
- **Transactions.** `begin_immediate` is `BEGIN IMMEDIATE`; `begin_read` is `BEGIN DEFERRED`, whose snapshot is held to commit. Savepoint names are quoted. Statements go through `prepare_cached`.
- **Errors.** SQLite failures become `Error::Sqlite(SqlError)` with the primary and extended result codes. A native table function that fails with a typed `tm_core::Error` has that error returned instead of a generic SQLite one.
- **Unsafe.** The crate denies `unsafe_code` and one private module, `table_fn`, opts out. rusqlite's `VTab` and `VTabCursor` traits are `unsafe` to implement, because SQLite reads the `#[repr(C)]` base structs directly, and a re-entrant table function needs `Connection::from_handle` to read through its calling connection. No raw pointer is dereferenced in this crate.
- **Bundled SQLite.** The `bundled` feature compiles SQLite from source, so a C compiler is needed to build, and results do not depend on the system library.

## Status

Version 0.1: the API may change between minor versions. MSRV is Rust 1.88. The crate is tested through the `tm-core` and `tiramemsu` suites rather than by tests of its own. Other hosts (WASM SQLite, Durable Objects) are only planned; they would be separate crates implementing the same traits.

Design and specs: <https://github.com/Volland/tiramemsu/blob/main/lat.md/architecture.md> (see "Executor"). Project home: <https://github.com/Volland/tiramemsu>. License: MIT OR Apache-2.0.
