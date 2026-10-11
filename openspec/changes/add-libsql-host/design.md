## Context

`tm-core` reaches SQLite only through the synchronous `Executor`/`Host` traits (`crates/tm-core/src/exec.rs`, `lat.md/architecture.md#Executor`). Two hosts exist: `tm-rusqlite` (`rusqlite` 0.40, bundled SQLite 3.53.0) and `tm-wasm` (the same `RusqliteExec` over `sqlite-wasm-rs`). **Deferred (2026-10-11).** This design was written when vector recall (M7b) was planned on libSQL's native vector search. Vector recall is now host-neutral (`add-vector-recall`: plain `embedding` table, exact search in Rust, every host), so libSQL is not the vector path and the native default does not move. The research below is kept: it picks the Rust binding, lists what `tm-rusqlite` actually uses and whether each candidate provides it, and describes how an optional `tm-libsql` host would be added and tested. If built, its use for vectors is limited to an optional approximate tier that follows `add-vector-recall`'s accelerator rules (derived from `embedding`, never authoritative, exact re-rank, `exact: false` when hits may be missed).

What `tm-rusqlite` uses today (read from `crates/tm-rusqlite/src/{lib,register,table_fn,error}.rs`):

- `Connection::open_with_flags` with `READ_WRITE | CREATE | NO_MUTEX | URI` (writer) and `READ_ONLY | NO_MUTEX | URI` (readers); `busy_timeout`; `prepare_cached` with `set_prepared_statement_cache_capacity(256)` and `flush_prepared_statement_cache`; `params_from_iter`, `ValueRef`, `changes()`; `execute_batch` for `BEGIN IMMEDIATE`/`BEGIN DEFERRED`/`COMMIT`/`ROLLBACK`/`SAVEPOINT`/`ROLLBACK TO`/`RELEASE`.
- `create_scalar_function` (flags `UTF8 | DETERMINISTIC | INNOCUOUS`), `create_aggregate_function` (the `Aggregate` trait), `UserFunctionError`.
- Virtual tables: `create_module` with an eponymous-only module, `VTab`/`VTabCursor`, `IndexInfo` (`constraints`, `constraint_usage`, `set_argv_index`, `set_omit`, `set_idx_num`, `set_estimated_cost`, `set_estimated_rows`), `VTabConnection::config(Innocuous)`, `Filters` in `xFilter`, `ModuleError`; `rusqlite::vtab::array::load_module` (`rarray`) and binding `Rc<Vec<Value>>` arrays.
- `unsafe Connection::from_handle(conn.handle())` for the non-owning connection that `tm_path`, `tm_text` and `tm_lftj` read through inside the calling statement.
- `progress_handler(1000, Some(..))` for budgets (`Executor::set_interrupt`); `PRAGMA compile_options` to declare `stat4`/`fts5`.
- SQL the engine issues through it: STRICT tables, `WITHOUT ROWID`, partial and expression indexes, triggers with `RAISE` (format 4 date guards), `PRAGMA journal_mode = WAL`, `synchronous = NORMAL`, `recursive_triggers = ON`, `schema_version`, `optimize = 0x10002`, full `ANALYZE`, FTS5 (`term_fts`, bm25), `json_each`/`json_group_array`, `sqlite_schema`, `NULLS FIRST/LAST`, `MATERIALIZED` CTEs, `sqlite_version()`.

## Goals / Non-Goals

**Goals**

- A libSQL host with the full capability set `tm-rusqlite` has (`reader_pool`, `functions`, `vtab`, `stat4`, `fts5`) plus `vector`, every flag established by running the feature.
- The full workspace suite, the SPARQL/Cypher differential suites, the feature matrix and the binding smoke tests pass on both native hosts before `host-libsql` is offered.
- File interchange in both directions while no vector index exists, and a specified, tested outcome when a stock-SQLite host opens a file that has one.
- An opt-in host with a written exit criterion; no default switch (withdrawn 2026-10-11).

**Non-Goals**

- Remote libSQL, embedded replicas, sync, Hrana, encryption (`multiple-ciphers`), WebAssembly UDFs.
- Using libSQL-only SQL (`ALTER TABLE ... ALTER COLUMN`, random rowid) anywhere in the engine. The only libSQL-specific SQL the engine may ever issue is the vector SQL of a future approximate tier, and only on hosts that declare `vector`.
- A libSQL WASM host.

## Decisions

### D1. Binding: `libsql-rusqlite` over `libsql-ffi` (bundled)

Candidates, checked on crates.io on 2026-10-10:

| Candidate | Latest | Sync API | UDFs / vtabs / `rarray` | Progress handler, interrupt | Bundled SQLite | FTS5 / STAT4 | Vector SQL | License |
|---|---|---|---|---|---|---|---|---|
| `libsql` | 0.9.30 (2026-03-19); 0.10.0-pre.4 (2026-06-02) | No: `execute`, `query`, `prepare`, `transaction` are `async` | No: no function or module registration, no raw handle | `interrupt()` only | via `libsql-sys` → `libsql-ffi` | yes | yes | MIT |
| `libsql-ffi` | 0.9.30; 0.10.0-pre.4 | raw C API | raw C API (`sqlite3_create_function_v2`, `sqlite3_create_module_v2`) | raw | libSQL 0.2.3 on SQLite **3.45.1** (0.9.30), **3.47.0** (0.10.0-pre.4) | `-DSQLITE_ENABLE_FTS5`, `-DSQLITE_ENABLE_STAT4` in `build.rs` | yes (unless `SQLITE_OMIT_VECTOR`) | MIT |
| `libsql-rusqlite` | 0.9.30 (2026-03-19); 0.10.0-pre.4 (2026-06-02), released in lockstep with `libsql` | Yes (rusqlite API) | Yes: `create_scalar_function`, `create_aggregate_function`, `create_module`, `eponymous_only_module()`, `vtab::array` (`rarray`), `VTabConfig::Innocuous` | `progress_handler`, `get_interrupt_handle` | through `libsql-ffi` | as `libsql-ffi` | yes (plain SQL) | MIT |
| `turso` / `turso_core` | 0.8.2 (2026-10-08), beta | async | **No**: `sqlite3_create_function*` and `sqlite3_create_module*` are unsupported in `COMPAT.md` | `sqlite3_progress_handler` step-time only | none (Rust rewrite) | **No FTS5** (Tantivy FTS instead); `PRAGMA recursive_triggers` unsupported | vector functions compatible with libSQL; DiskANN index not listed | MIT |
| `rusqlite` + `sqlite-vec` (the earlier roadmap plan) | 0.40.2 + 0.1.9 | yes | yes | yes | SQLite 3.53.0 | yes | `vec0` virtual table, brute-force scan, no ANN index | MIT/Apache-2.0 |

Chosen: **`libsql-rusqlite` (with `bundled`, `functions`, `vtab`, `array`, `hooks`) over `libsql-ffi`**, pinned to an exact version. It is the only candidate that is synchronous and exposes everything the host uses. `libsql` is rejected because it is async-only and cannot register functions or virtual tables, which `tm-exec` requires (`check_capabilities` refuses to open without `functions` and `vtab`); blocking on its futures would add a runtime and still lack the registry. Raw `libsql-ffi` is rejected because it would re-implement rusqlite's safe wrappers in new `unsafe` code. Turso is rejected for now: no user functions, no virtual tables and no FTS5 means it could only be a `tm-core`-tier host, the same as Cloudflare Durable Objects; revisit when its C API gains `create_function`/`create_module`.

Rejected alternative for code layout: a cargo feature inside `tm-rusqlite` that swaps the `rusqlite` dependency for `libsql-rusqlite`. Feature unification would make `tm-rusqlite` mean two different SQLite builds depending on who else is in the graph, and `tm-wasm` depends on `tm-rusqlite`. A separate crate keeps each host's identity fixed.

### D2. The API gap is small and mechanical

`libsql-rusqlite` is a fork of an older rusqlite (edition 2018, pre-0.32 API). Against the `rusqlite` 0.40 calls above the differences are:

| Used by `tm-rusqlite` | `libsql-rusqlite` 0.9.30 | Port |
|---|---|---|
| `Module::eponymous_only_module()` (const) | `vtab::eponymous_only_module::<T>()` returns `&'static Module` | call the function |
| `VTabCursor::filter(.., args: &Filters)` | `args: &Values` | rename the type |
| `progress_handler(..)` returns `Result` (0.40); host keeps the interrupt only on `Ok` | returns `()` | always installed on an owning connection; the borrowed handle never installs one (unchanged rule) |
| `Connection::from_handle` | present (`unsafe`) | same |
| `prepare_cached`, cache capacity, flush | present | same |
| `create_scalar_function`, `create_aggregate_function`, `FunctionFlags::SQLITE_INNOCUOUS` | present | same |
| `vtab::array::load_module`, `rarray` binding | present | same |
| `VTabConfig::Innocuous` | present | same |
| `Error::SqliteFailure`, `UserFunctionError`, `ModuleError` | present | same `map_err` |

`tm-libsql` is a port of `tm-rusqlite` (about 850 lines, of which `table_fn.rs` is the only `unsafe`), with these edits. The two crates share a host conformance test suite (D6), not source files.

### D3. Capability probing, including `vector`

`LibsqlHost::new()` probes on a scratch in-memory connection, like `tm-wasm`'s `probe_capabilities`, instead of trusting `compile_options` alone:

- `functions`: a registered scalar runs. `vtab`: `rarray` answers a bound array. `fts5`: an FTS5 table is created and matched. `stat4`: `PRAGMA compile_options` lists `ENABLE_STAT4`. `reader_pool`: true (file VFS, `THREADSAFE=1`).
- `vector`: `CREATE TABLE p(id INTEGER PRIMARY KEY, e F32_BLOB(3))`, `CREATE INDEX p_i ON p(libsql_vector_idx(e, 'metric=cosine'))`, insert two rows with `vector32('[...]')`, and `SELECT id FROM vector_top_k('p_i', vector32('[...]'), 1)` returns the nearest id, and `vector_distance_cos` returns a finite REAL. Any failure leaves `vector = false`.
- `LibsqlHost::runtime_info()` reports `sqlite_version()`, `libsql_version()` when present, `compile_options` and the probed set; `limit_capabilities` narrows the set for tests, as on `tm-wasm`.

`Capabilities` gains `pub vector: bool`; `RusqliteHost` and `WasmHost` declare `false`. The struct is `Copy + Default`, so adding a field is source-compatible for code that uses `..Default::default()`; struct literals in the workspace are updated.

### D4. One native host per binary

`libsql-ffi` declares no `links` key and compiles `sqlite3.c` with unprefixed `sqlite3_*` symbols, exactly as `libsqlite3-sys` (bundled) does. Linking both into one binary gives duplicate symbols at best and silently mixed implementations at worst. Therefore:

- The facade gets features `host-rusqlite` and `host-libsql`; `Db::open` uses the selected host (`tiramemsu::NativeHost` alias). Enabling both is a `compile_error!` naming the conflict; enabling neither leaves only `Db::open_with_host`.
- Cross-host tests run as two processes: each build writes and reads files, a script hands them over (as `scripts/wasm-interop.sh` does for WASM). No test links both hosts.
- Phase 1 verifies the conflict on Linux, macOS and Windows; if the libraries can coexist (for example through symbol prefixing upstream), the rule can be relaxed in a later change, not silently.

Alternative considered: link both and pick at runtime. Rejected for the reason above; it also doubles binary size for bindings.

### D5. SQLite version and behavior differences

libSQL lags upstream SQLite: `libsql-ffi` 0.9.30 bundles SQLite 3.45.1, the 0.10 pre-releases 3.47.0, against 3.53.0 in `rusqlite` 0.40. The engine's SQL was audited for features newer than 3.45 (STRICT 3.37, `RAISE` triggers, `sqlite_schema` 3.33, `NULLS FIRST/LAST` 3.30, `MATERIALIZED` 3.35, JSON functions, FTS5 bm25 all predate it). One real gap was found:

- **`PRAGMA optimize = 0x10002`.** The `0x10` and `0x10000` mask bits ("analyse tables never analysed", "check all tables") were added in SQLite 3.46.0. In 3.45.1's `sqlite3.c` only `0x01` and `0x02` are tested, so on `libsql-ffi` 0.9.30 the open-time optimize does nothing for never-analysed tables and the `storage-format` requirement "Planner statistics are part of the store" would not hold. Decision: require the libSQL release that bundles SQLite ≥ 3.46 (0.10.x, 3.47.0) and, independently, make the host assert at open that the statistics requirement held (`sqlite_stat1` has rows for `triple` after the open-time upkeep on a non-empty file), falling back to an explicit table-scoped `ANALYZE` when it did not. Pinning 0.10 while it is a pre-release is a phase-1 risk (see Risks).
- **Build defaults.** `libsql-ffi` compiles with `SQLITE_DEFAULT_FOREIGN_KEYS=1`. The schema has no foreign keys, so this is inert; the host still sets `PRAGMA foreign_keys` explicitly to the stock default so a future schema cannot behave differently per host.
- **Plans.** `lat.md/storage.md` records plans verified on 3.53. The plan-quality fixture (`lat.md/roadmap.md#Benchmarks`) runs on both hosts in phase 1; a plan that changes is recorded, and results must be equal regardless (statistics never change results).
- **`sqlite_version()`** reports the bundled SQLite version on libSQL as well; diagnostics add `libsql_version()`.

### D6. Test matrix (phase 1)

- A host conformance suite (`tests/host_conformance.rs` pattern, parameterised by a `Host` constructor) runs the required-operations scenarios of `sql-executor`, every capability probe, interrupt, borrowed-connection reads inside a table function, the typed-error slot, and registration on readers.
- CI job `hosts` with matrix `host: [rusqlite, libsql]` × `os: [ubuntu, macos, windows]` runs `cargo test --workspace --no-default-features --features host-$host,sparql,cypher` (bindings and MCP included through their own features), the SPARQL and Cypher differential suites, the TCK harness counts (must be identical), the feature matrix's core/exec/sparql/cypher combinations, and the Python/Node smoke tests.
- CI job `host-interchange`: build both, then for each direction write a fixture covering every format-4 feature (terms of every tag, statements, retractions with every `ret_kind`, graphs, valid time, volatile rows, saved answers, the text index), close, open with the other build, compare every engine row and counter (`meta`, `term`, `tx`, `triple`, `volatile`, `pred_multi`, `saved_answer*`), run recall and query fixtures, write more, and hand it back.
- The `tm-core` dependency check in `ci.yml` also rejects `libsql`, `libsql-ffi`, `libsql-sys`, `libsql-rusqlite` in `cargo tree -p tm-core -e normal`.

### D7. File compatibility

- **Without a vector index** the file format is the same: libSQL reads and writes the SQLite 3 file and WAL formats unchanged unless libSQL-specific features (virtual WAL, random rowid, encryption) are enabled, and the host enables none. Interchange is tested in both directions (D6), including the WAL left by an uncleanly closed writer.
- **With a vector index** (only if a future approximate tier creates one on a `vector` host; `add-vector-recall` itself creates none): the derived ANN tables (`vec_*` names reserved by format 1) carry an index whose expression calls `libsql_vector_idx`, plus libSQL's shadow tables. Stock SQLite opens such a file and reads every table, but any statement that evaluates that index expression fails with "unknown function: libsql_vector_idx()": a write to the indexed table, a full `ANALYZE`, `PRAGMA integrity_check`, `VACUUM`, `REINDEX` (observed with a stand-in function on SQLite 3.53.0; `PRAGMA quick_check`, reads, writes to other tables, `PRAGMA optimize` and `DROP TABLE` of the ANN table succeed). Decision: **open, never refuse, and never touch**. A stock-SQLite host opens the file normally, declares `vector = false`, never reads or writes `vec_*` tables, and runs statistics table by table over the engine's own tables instead of a schema-wide `ANALYZE` (to be specified by the tier's change, where the tables appear). Exact vector recall keeps working on that host from the `embedding` table; only the approximate tier is unavailable. Dropping the derived ANN tables on any host returns the file to plain SQLite for every tool.
- Rejected alternative: refuse such files on stock hosts with a typed error. It would lock a user's memory to one SQLite build for the sake of a derived, rebuildable index, and the never-touch rule is the same one FTS5-less hosts already follow for `term_fts`.

### D8. WebAssembly keeps stock SQLite

`tm-wasm` runs `RusqliteExec` over `sqlite-wasm-rs` on `wasm32-unknown-unknown`. libSQL offers no equivalent Rust path: `libsql-ffi`'s build supports `wasm32-wasi` (`SQLITE_OS_OTHER` plus an optional WASI VFS), not the browser target, and the `libsql` crate's `wasm` feature is the remote Hrana client, not an embedded engine. An experimental JavaScript build (`@libsql/libsql-wasm-experimental`) exists but cannot be driven synchronously from the Rust host inside the worker. Decision: `tm-wasm` stays on stock SQLite and never declares `vector` (libSQL vector SQL); the WASM binding's capability list omits it. Exact vector recall (`add-vector-recall`) still works in the browser because it needs no capability. Revisit if a `wasm32-unknown-unknown` libSQL build with vectors appears; the probe would then declare it without any engine change.

Alternative considered: compile `libsql-ffi`'s amalgamation through `sqlite-wasm-rs`'s toolchain ourselves. Rejected: a private fork of a fork, with no upstream testing on that target.

### D9. Phases and exit criteria

1. **Opt-in (the only phase while deferred).** `tm-libsql` published; facade `host-libsql` opt-in. Exit: D6 green for four consecutive weeks on all three OSes; TCK and differential counts identical; interchange green both ways; release benchmarks (churn, point/2-hop, path, supersede, bulk import, text recall) within 10 % of `rusqlite` or the difference explained; the 0.10 libSQL line has a stable (non-pre) release or the project accepts the pin explicitly.
2. **Default (withdrawn 2026-10-11; kept for reference, would need a new change).** Flip defaults in the facade, JSON bridge, MCP server, Python wheels and Node addon; `host-rusqlite` remains a documented feature; release notes call out the SQLite version change (3.53 → libSQL's) and the build-time C toolchain requirement (unchanged in kind: both compile `sqlite3.c`). Exit: one release in the field without a host-attributed regression.
3. **Decide (withdrawn 2026-10-11).** Keep `tm-rusqlite` as a supported native fallback (CI matrix stays) or reduce it to the WASM base (native CI only through `tm-wasm`'s `Storage::File`). Inputs: libSQL release cadence versus upstream SQLite, the Turso rewrite's progress, user demand for stock SQLite, and CI cost.

## Risks / Trade-offs

- **Upstream direction.** Turso's own README says new features go into Turso (the Rust rewrite), and libSQL is "actively maintained" but no longer where development happens. libSQL tracks upstream SQLite slowly (3.45.1 in the latest stable crate, 3.47.0 in pre-releases, while SQLite is at 3.53). Mitigation: keep `tm-rusqlite` building and tested through phase 3; the executor boundary makes a later move to Turso, or back, one crate; vectors are a capability, so a host without them still runs everything else.
- **Pre-release pin.** The SQLite ≥ 3.46 requirement (D5) currently means `libsql-rusqlite` 0.10.0-pre.x. Mitigation: the table-scoped `ANALYZE` fallback makes 0.9.30 workable if 0.10 stalls; phase 2 waits for a stable release or an explicit decision.
- **Fork API drift.** `libsql-rusqlite` follows an older rusqlite; future rusqlite fixes (vtab safety, `progress_handler` errors) do not arrive automatically. Mitigation: the host's `unsafe` surface stays confined to `table_fn.rs` and is reviewed against both forks.
- **Symbol conflict.** Users who link another crate that bundles SQLite (`rusqlite`, `sqlx-sqlite`) next to `tiramemsu` with `host-libsql` will hit duplicate symbols. Mitigation: documented in the facade README; `host-rusqlite` remains for them.
- **Behavior drift between hosts.** Different SQLite versions may plan differently or differ in edge semantics (floating formatting, `json_*`). Mitigation: the full differential suites run on both hosts; any difference is a failing test, not a footnote.
- **Binary size and build time** grow for bindings by the libSQL additions (vector code, sqlean if ever enabled). Measured in phase 1 and reported.

## Migration Plan

If the deferral is lifted, phase 1 ships behind `host-libsql` with no default change and no format change; files need no migration because the format is identical. Rollback is removing the feature. A default switch is out of scope and would be a separate change.

## Open Questions

- Pin `libsql-rusqlite` 0.10.0-pre.x for SQLite 3.47, or ship 0.9.30 (3.45.1) with the table-scoped `ANALYZE` fallback and move later?
- Does `libsql-ffi` create `libsql_vector_meta_shadow` eagerly on a fresh file? If it does, the "empty file is initialised" rule of `storage-format` must accept that table (phase-1 probe).
- Python wheels and the Node addon: one artifact per host, or only the default host per release?
- Phase 3: is a stock-SQLite native fallback worth a permanent CI matrix?

## Sources

- crates.io API, `libsql`, `libsql-ffi`, `libsql-sys`, `libsql-rusqlite`, `turso`, `turso_core`, `rusqlite`, `sqlite-vec` (versions, dates, features, licenses), queried 2026-10-10: https://crates.io/crates/libsql, https://crates.io/crates/libsql-rusqlite, https://crates.io/crates/libsql-ffi, https://crates.io/crates/turso
- `libsql-ffi` 0.9.30 and 0.10.0-pre.4 crate sources: bundled `sqlite3.h` (`SQLITE_VERSION` 3.45.1 / 3.47.0, `LIBSQL_VERSION` 0.2.3), `build.rs` compile flags (FTS5, STAT4, `DEFAULT_FOREIGN_KEYS=1`, `THREADSAFE=1`), `PRAGMA optimize` mask handling in `sqlite3.c`; https://github.com/tursodatabase/libsql/blob/main/libsql-ffi/build.rs
- `libsql-rusqlite` 0.9.30 crate source (API: `eponymous_only_module`, `Values`, `progress_handler`, `from_handle`, `vtab::array`).
- libSQL README (libSQL vs Turso, maintenance note): https://github.com/tursodatabase/libsql
- `libsql::Connection` API (async methods, no function or module registration): https://docs.rs/libsql/0.9.30/libsql/struct.Connection.html
- Turso vector search docs (`F32_BLOB`, `vector32`, `libsql_vector_idx` settings, `vector_top_k`, automatic index upkeep, limits): https://docs.turso.tech/features/ai-and-embeddings
- Turso `COMPAT.md` (no `sqlite3_create_function`, no `sqlite3_create_module`, no FTS5, `recursive_triggers` unsupported, vector functions compatible with libSQL, SQLite file format supported): https://github.com/tursodatabase/turso/blob/main/COMPAT.md, https://docs.turso.tech/sql-reference/compatibility
- Stock-SQLite behavior with an index on an unknown function: local experiment with Python's `sqlite3` (SQLite 3.53.0) and a stand-in `libsql_vector_idx`; to be confirmed against a real libSQL-built file in task 1.4.
