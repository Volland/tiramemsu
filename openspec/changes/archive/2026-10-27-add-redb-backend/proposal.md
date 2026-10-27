# redb storage backend

Target release: **0.8 — Two engines**. Second of six changes (D37–D44 in `lat.md/overview.md#Decision Record`). Depends on `add-store-trait`.

## Why

SQLite gives the engine its planner, triggers and FTS5, but every read pays for the SQL layer, a read needs a pooled connection, and the cyclic-join operator has to fetch its ranges through SQL ([[query#Physical Planning#LFTJ]]). redb (4.4, pure Rust, copy-on-write B-trees, MVCC readers, ordered typed keys) can hold the same logical database as sorted permutations that a native runner reads directly, with as many concurrent readers as there are threads.

The storage research of 2026-10 (`docs/storage-engine-research.md`) records the expected costs: write and space amplification of a copy-on-write B-tree, and point reads no faster than SQLite's. This change builds the backend so that those costs can be measured (`add-backend-benchmarks`), not assumed.

## What Changes

- **New crate `tm-redb`** implementing `ReadStore`/`WriteStore` on redb, with the layout below. The engine is `tm-store`'s, unchanged.
- **Layout (redb format 1):** `stmt` (eid → record), six live permutation tables (`l_spo`, `l_sop`, `l_pso`, `l_pos`, `l_osp`, `l_ops`; fixed 32-byte big-endian keys `(a, b, c, eid)`, value `(v_from, v_to)`), three history tables (`h_spo`, `h_pos`, `h_osp`; key `(a, b, c, !t_add, eid)`, value `(t_ret, v_from, v_to)`), the log (`log_add`, `log_ret`), `term` and `term_key`, `term_num`, `tx` and `tx_instant`, `meta`, `pred_multi`, `pred_stats` (per predicate: live count, distinct live subjects, distinct live objects), `volatile`, `text_*` (inverted index), `embedding`.
- **Exact predicate statistics** maintained inside each write transaction.
- **Text index in redb tables**, tokenised like FTS5 `unicode61 remove_diacritics 2`, scored with FTS5's BM25 constants, kept in the same transaction as the statement, so recall returns the same hits in the same order on both backends.
- **Backend selection:** `Db::open` detects the backend from the file's magic bytes; `OpenOptions::backend = Backend::{Auto, Sqlite, Redb}`; `Db::create(path, backend, opts)`; errors `UnknownFormat`, `BackendMismatch`, `BackendNotCompiled`. New files default to SQLite.
- **Durability modes for both backends:** `OpenOptions::durability = Durability::{Normal, Immediate, None}` with group commit on redb.
- **Facade dispatch:** `Db`, `View` and `Tx` route to the SQLite engine or to the `tm-store` engine over redb. Every core API works on redb (transactions, speculation, dry runs, views, `triples`, events, dependents, bundles, conflicts, text and vector recall, volatile state). SPARQL and Cypher on a redb file return `MissingCapability("native_runner")` until `add-native-runner`.
- **Conversion and maintenance:** `tiramemsu::convert(src, dst, backend)` copies a database losslessly (same eids, clocks, terms, counters, burned ids); `Db::compact()` reclaims redb space.
- **Cargo feature `redb`** on the facade (default off for the Rust crate, on for the Node, Python and CLI builds).

## Capabilities

### New Capabilities

- `redb-storage`: the redb layout, statistics, text index, concurrency, conversion and compaction.
- `backend-selection`: detection, the `backend` option, `Db::create` and the related errors.
- `durability-modes`: the `durability` option on both backends.

### Modified Capabilities

None; SQLite behaviour is unchanged when `durability` is left at `Normal`.

## Impact

- **New crate:** `crates/tm-redb`.
- **Facade:** `crates/tiramemsu/src/db.rs`, `view.rs`, `bundle.rs`, `review.rs` (dispatch), new `backend.rs` (detection, `Backend`, `create`, `convert`), `Cargo.toml` (`redb` feature).
- **SQLite host:** `crates/tm-rusqlite` maps `Durability` to `synchronous`.
- **Bindings:** JSON bridge open options `backend`, `durability`; ops `create`, `compact`; Node and Python options.
- **lat.md:** `storage.md` (new section redb Layout), `architecture.md#Storage Backends`, `api.md#Open Options`, `tests.md`.
- **Decisions that bind the area:** D9, D10, D17, D25, D35 and the new D38–D41.
- **Requirements added:** [[openspec:redb-storage#Same logical database]], [[openspec:redb-storage#Permutation layout]], [[openspec:redb-storage#Exact predicate statistics]], [[openspec:redb-storage#Text index parity]], [[openspec:redb-storage#Readers without a pool]], [[openspec:redb-storage#Lossless conversion]], [[openspec:redb-storage#Compaction]], [[openspec:backend-selection#Detection by file header]], [[openspec:backend-selection#Explicit backend choice]], [[openspec:backend-selection#Creating a database]], [[openspec:backend-selection#Backend compiled out]], [[openspec:durability-modes#Durability option]].

## Non-goals

- No query execution on redb (that is `add-native-runner`); no CLI (`add-tm-cli`).
- No WASM or Durable Objects support for redb.
- No change to the SQLite file format.
- No default change: SQLite stays the backend of new files.
