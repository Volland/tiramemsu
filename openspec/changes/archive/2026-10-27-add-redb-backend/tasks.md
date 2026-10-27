## Status

Implemented 2026-10-27. Depends on `add-store-trait`. See the Resolution in `design.md`.

## 1. Crate and layout

- [x] 1.1 Create `crates/tm-redb` (redb 4.4); byte keys compared with `memcmp`, integers encoded big-endian with the sign bit flipped (design D-1).
- [x] 1.2 Tables of design D-2; `info` with `engine`, `layout` — annotated `Same logical database`, `Permutation layout`.
- [x] 1.3 `ReadStore` on `ReadTransaction` (`RedbRead`) and `WriteStore` on `WriteTransaction` (`RedbWrite`); permutation choice per bound positions; unbound scans in eid order from `stmt`.
- [x] 1.4 `pred_stats` upkeep in the write transaction and `count` from it — annotated `Exact predicate statistics`.
- [x] 1.5 Text index tables (`text_doc`, `text_token`, `text_post`), the FTS5-compatible tokeniser and `bm25()` (`tm_store::search`) — annotated `Text index parity`.
- [x] 1.6 Embedding table; vector search, `unembedded` and hybrid recall over the traits (`tm_store::search`).

## 2. Facade

- [x] 2.1 `Backend`, detection, `OpenOptions::backend`, `Db::create`, `Db::backend()`, errors `UnknownFormat`, `BackendMismatch`, `AlreadyExists`, `BackendNotCompiled` — annotated for the four `backend-selection` requirements.
- [x] 2.2 `Writer::{Sql, Kv}` dispatch; `View::either` routes every core read; readers are snapshots — annotated `Readers without a pool`. The facade `Tx` is now `tm_core::TxHandle` on both engines.
- [x] 2.3 `Durability`, group-commit flusher, `Db::sync()`, SQLite `synchronous` mapping — annotated `Durability option`.
- [x] 2.4 `convert` (through `tm_core::sqlstore::SqlStore` and `tm_store::copy`) and `Db::compact()` — annotated `Lossless conversion`, `Compaction`.
- [x] 2.5 `redb` cargo feature on the facade; queries on redb return `MissingCapability("native_runner")` until `add-native-runner`.

## 3. Bindings

- [x] 3.1 JSON bridge: open options `backend`, `durability`, `flushIntervalMs`, `flushCommits`, `create`; ops `sync`, `compact`; `info.backend`; error codes; Node and Python options, `sync`, `compact`, `backend`; the bridge builds with `redb`.

## 4. Tests

- [x] 4.1 `crates/tiramemsu/tests/backends.rs` (one test per scenario, crash tests in a child process), `backend_not_compiled.rs`, the layout unit test in `tm-redb`, the bridge test.
- [x] 4.2 The `tm-store` contract and oracle (now with recall parity) run on `tm-redb`.

## 5. Documentation and checks

- [x] 5.1 lat.md: `storage.md` section redb Layout with Conversion, `architecture.md` Write Handle, `api.md#Open Options`; decisions D38–D41 in `backends.md`.
- [x] 5.2 Workspace tests (1 387 passed before the bridge change, plus the new suites), clippy, `tg check`, `tg ontology check`, `tg trace` for the three capabilities, `openspec validate add-redb-backend`.
