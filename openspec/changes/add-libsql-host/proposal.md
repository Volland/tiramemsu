# libSQL host

## Status

Deferred (2026-10-11): not the vector path; an optional native host that may later offer an approximate tier.

Proposed 2026-10-10, when vector recall was planned on libSQL's native vector indexes. The decision of 2026-10-11 (user-approved) made vector recall host-neutral instead (`add-vector-recall`: a plain SQLite `embedding` table and exact, view-filtered search in Rust, on every host). This change is therefore not required for vectors and does not migrate the default host. It is kept as research and as the plan for an optional `tm-libsql` host, which could later provide an approximate accelerator tier under the rules of `add-vector-recall` (design, Future work): derived from `embedding`, never authoritative, exact re-rank.

## Why

Agents recall by meaning. `docs/feature-roadmap.md` (0.6, Agent recall) planned vector recall through `sqlite-vec` on hosts that probe it. libSQL was evaluated as the vector path: its `F32_BLOB(n)` columns, `vector_distance_cos`, `libsql_vector_idx(...)` DiskANN indexes and `vector_top_k(idx, q, k)` table function are compiled into the engine: an approximate nearest-neighbour index that lives in the same file, inside the same transactions, with no loadable extension to ship per platform. That plan was replaced on 2026-10-11 by host-neutral exact search, because approximate indexes filter after top-k and lose recall under as-of, valid-time and graph views, and because libSQL cannot run in WASM. libSQL remains interesting as an optional native host and a possible approximate tier.

The executor boundary (`lat.md/architecture.md#Executor`) was drawn so a second SQLite build costs one host crate. The WASM host proved it for SQLite compiled to WebAssembly; libSQL is the second native build. What it needs is an honest capability declaration, a test matrix that runs every suite on both hosts, and file interchange in both directions.

## What Changes

- New crate `tm-libsql`: `LibsqlHost` and `LibsqlExec` implementing `Host` and `Executor` over `libsql-rusqlite` (the rusqlite fork on `libsql-ffi`, bundled C), with the same registration glue as `tm-rusqlite` (scalar and aggregate functions, eponymous virtual tables, the connection-aware table functions, `rarray`, the progress-handler interrupt).
- New capability flag `Capabilities::vector`, established by probing (create an `F32_BLOB` table, a `libsql_vector_idx` index, answer `vector_top_k`), declared only by `tm-libsql`. Every other flag is probed the same way rather than read from a version number.
- Facade cargo features `host-rusqlite` and `host-libsql` select the native host linked into a binary; exactly one is linked (the two bundled C libraries export the same `sqlite3_*` symbols).
- **Phase 1 (opt-in):** `host-rusqlite` stays the default; `host-libsql` is opt-in; CI runs the full workspace suite, the differential suites, the feature matrix and the bindings smoke tests against both hosts, and an interchange job hands files between a rusqlite build and a libSQL build in both directions.
- **No default switch.** `host-rusqlite` stays the default native host. The earlier phase 2 (make `host-libsql` the default) and phase 3 (reduce `tm-rusqlite`) are withdrawn; a default change would need a new, separate change.
- File compatibility: a tiramemsu file moves between stock SQLite and libSQL unchanged (format 5's `embedding` table is plain SQLite). If a future approximate tier adds libSQL vector index tables, stock-SQLite hosts keep opening the file, never touch those derived tables and never run schema-wide statements that would evaluate the libSQL index expression (specified here as the host contract).
- `tm-wasm` keeps stock SQLite (`sqlite-wasm-rs`); no Rust-usable libSQL WebAssembly build exists for `wasm32-unknown-unknown`, so `vector` (libSQL vector SQL) is never declared there; vector recall itself still works in WASM because it needs no capability.

## Capabilities

### New Capabilities

- `libsql-host`: the optional libSQL executor host, its probed capabilities, host selection and file interchange with stock-SQLite hosts.

### Modified Capabilities

- `sql-executor`: `Capabilities` gains `vector` (libSQL vector SQL, for an optional approximate tier; vector recall does not require it); the core's dependency rule names the libSQL crates too; the rusqlite host stays the default but is no longer the only native host.
- `wasm-sqlite-host`: the WASM host never declares `vector`.

## Impact

New crate `crates/tm-libsql`; `tm-core` (`Capabilities::vector`, no new dependency); facade `Cargo.toml` features and `Db::open` host selection; `tiramemsu-json`, `tiramemsu-mcp`, `bindings/python`, `bindings/node` host features (defaults unchanged); `.github/workflows/ci.yml` host matrix and interchange job; `scripts/feature-matrix.sh` gains a host axis. No storage-format change in this change. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior.

## Non-goals

- No vector API, table or format bump here (that is `add-vector-recall`, which does not depend on this change).
- No default-host switch.
- No libSQL server, embedded replicas, Hrana or sync; the host opens local files only.
- No Turso (the Rust rewrite) host in this change; it lacks user functions and virtual tables today (see `design.md`).
- No change to the WASM runtime.
