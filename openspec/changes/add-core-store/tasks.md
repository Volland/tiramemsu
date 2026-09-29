## 1. Workspace and CI

- [ ] 1.1 Create the Cargo workspace root `Cargo.toml` (`members = ["crates/*"]`, edition 2021, shared `[workspace.lints]` with `unsafe_code = "forbid"`, `clippy::all = "warn"`), `rust-toolchain.toml` (stable) and `.gitignore` (`target/`, `*.db`, `*.db-wal`, `*.db-shm`)
- [ ] 1.2 Create crate `crates/tm-core` (lib) with dependencies `thiserror`, `lru` and no SQLite binding; dev-dependencies `proptest`, `tempfile`, `tm-rusqlite`, `rusqlite` (raw-connection tests only); empty module files per design D-1; `cargo build` passes
- [ ] 1.3 Create crate `crates/tm-rusqlite` (lib) depending on `tm-core` and `rusqlite` (features `bundled`, `functions`, `vtab`, `array`), and crate `crates/tiramemsu` (lib) depending on `tm-core` and `tm-rusqlite`; dev-dependencies `proptest`, `tempfile`; `pub use tm_core::{...}` placeholder; `cargo build` passes
- [ ] 1.4 Add CI workflow (`.github/workflows/ci.yml`) running `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace` on Linux and macOS; add `rustfmt.toml`
- [ ] 1.5 Add a CI step that fails if engine source contains `DELETE FROM triple`, `DELETE FROM term`, `DELETE FROM tx`, `UPDATE term` or `UPDATE tx` (grep over `crates/*/src`, excluding test fixtures)
- [ ] 1.6 Add `crates/tm-core/tests/common/mod.rs` with `TestDb` (tempfile path, `ManualClock`, host parameter, helpers `iri()`, `lit()`, `assert_ok()`), used by all integration tests
- [ ] 1.7 Add a CI step that fails if `cargo tree -p tm-core -e normal` lists `rusqlite` or `libsqlite3-sys`

## 2. Identifiers, errors and the executor

- [ ] 2.1 Implement `id.rs`: `Tag` (0–14, `TryFrom<u8>` rejecting 15 `SEALED` with `Unsupported`), `ObjectId` with `tag()`, `signed_payload()`, `unsigned_payload()`, `from_signed()`, `from_unsigned()`, `Eid`, `TxId`; unit tests for the bit layout scenarios of `object-encoding` "ObjectId layout"
- [ ] 2.2 Implement `error.rs`: the `Error` enum of design D-3 (`#[non_exhaustive]`, `thiserror` messages, `Position` enum, `Unsupported { feature }`, host-neutral `Sqlite(SqlError)`), `Result<T>` alias
- [ ] 2.3 Implement `vocab.rs`: `sys:`, `tm:`, `xsd:`, `rdf:` IRI constants, tag IRIs `sys:<TAG>`, the reserved-namespace allow-list data, the reserved `sys:sensitive` IRI, skolem prefixes
- [ ] 2.4 Implement `exec.rs` per design D-17: `Executor` (prepared statements with bound params, `execute_batch`, `begin_immediate`, `begin_read`, `commit`, `rollback`, savepoints), `Host`, `HostOptions`, `Capabilities` (`reader_pool`, `functions`, `vtab`, `stat4`, `fts5`), `SqlValue`, `SqlError`; add `// @lat: [[architecture#Executor]]` on the trait
- [ ] 2.5 Implement `tm-rusqlite` `RusqliteHost` / `RusqliteExec`: `prepare_cached`, parameter binding, transactions and savepoints, `busy_timeout`, `rusqlite::Error` → `SqlError`, capabilities all `true` after checking `PRAGMA compile_options` for `ENABLE_STAT4` and `ENABLE_FTS5`, and an empty UDF / virtual-table registration hook for M1
- [ ] 2.6 Implement `MinimalHost` in `crates/tm-core/tests/common/` (wraps `RusqliteExec`, declares every capability `false`, records every prepared SQL string, can inject a host error) and make `TestDb` run each integration test on both `RusqliteHost` and `MinimalHost` (a macro that expands one test per host)
- [ ] 2.7 Tests for `sql-executor` "Required executor operations" (same-transaction reads, savepoint rollback, stable read snapshot), "Capabilities are declared by the host" (no reader pool on `MinimalHost`, no STAT4) and "The rusqlite host" (all capabilities, compile options, no `rusqlite` type in `tm-core`'s public items)

## 3. Values and canonical encoding

- [ ] 3.1 Implement `value.rs` `Value` and `Value::literal(lex, datatype, lang)` for `xsd:integer` (60-bit range → `Int`, else `Typed` canonical decimal), `xsd:boolean`, plain / `xsd:string`, `rdf:langString` (lower-cased tag), fallback `Typed`
- [ ] 3.2 Add `xsd:dateTime` parsing into `DateTime { ms, tz }` (offset kept as a timezone code, `Z` = `+00:00`, no timezone = code 0, offsets beyond ±14:00 and instants beyond ±2^48 ms → `Typed`, sub-ms truncation) and `xsd:date` parsing (days since epoch, timezone ignored)
- [ ] 3.3 Add `xsd:double` (canonical `1.0E0` form, NaN) and `xsd:decimal` (canonical form) canonicalisation; ill-typed lexical forms of all special datatypes → `Typed` verbatim
- [ ] 3.4 Implement `codec.rs` inline encode/decode for `NODE`, `BNODE`, `STMT`, `TX`, `INT`, `BOOL`, `DATETIME` (`(epoch_ms << 11) | tz`, decode with its own offset, `instant(id) = id >> 15`), `DATE`, `SHORT_STR` (big-endian bytes + 4-bit length, design D-4)
- [ ] 3.5 Implement skolem IRI export and recognition (`urn:tiramemsu:node:<n>` / `bnode:<n>`, canonical decimal only)
- [ ] 3.6 Unit tests for every scenario of `object-encoding` requirements "Canonical encoding of integers", "…booleans, dates and date-times", "…strings", "…doubles, decimals and other datatypes" and "Skolem IRIs for anonymous nodes"
- [ ] 3.7 Property test: encode → decode → encode is exact for random values of every tag and inlineable values never touch the dictionary; add `// @lat: [[tests#ObjectId#Canonical Round Trip]]` on the test
- [ ] 3.8 Property test: for random `INT` and `DATE` pairs, `a < b ⇔ oid(a) < oid(b)` (signed); for random `DATETIME` pairs with random offsets, instant order `⇔ oid >> 15` order; including negatives and range boundaries, also checked through a SQLite `ORDER BY` and SQLite's `>>`; add `// @lat: [[tests#ObjectId#Order Within Tag]]`
- [ ] 3.9 Tests for `object-encoding` date-time scenarios (offsets kept as two terms, same instant equal by `id >> 15` in Rust and SQLite, no-timezone round trip, out-of-range → `TYPED`); add `// @lat: [[tests#ObjectId#DateTime Keeps Its Offset]]` on the two-offsets test

## 4. Storage format

- [ ] 4.1 Add `storage/ddl_v1.sql` with the verbatim DDL, triggers and `event` view from `lat.md/storage#Schema`, `#Event View` and `#Invariant Triggers`; add `// @lat: [[storage#Schema]]` on the constant that includes it
- [ ] 4.2 Implement `storage::open` over a `Host`: pragmas (WAL, `synchronous=NORMAL`, `recursive_triggers=ON`; `busy_timeout` through `HostOptions`), fresh-file initialisation in one transaction with initial `meta` rows, empty-file initialisation, `ForeignFile` detection, then `PRAGMA optimize=0x10002` (design D-6, D-18)
- [ ] 4.3 Implement `storage/meta.rs` `Counters` load/store and `storage/migrate.rs` (`Migration`, empty `MIGRATIONS`, version check → `FormatVersion`, ordered forward migration in one transaction, test-only `open_with`)
- [ ] 4.4 Tests for `storage-format` "Opening creates a new database file", "Tables are STRICT and the journal is WAL", "Schema matches format version 1 exactly" (inspect `sqlite_schema` / `pragma index_xinfo`) and "Engine metadata counters" (fresh values)
- [ ] 4.5 Tests for "Newer format versions are refused" (file bytes unchanged), "Older format versions are migrated forward atomically" (synthetic v1→v2 migration, failing migration rolls back) and "Foreign files are refused"
- [ ] 4.6 Test "Interrupted creation leaves no half-initialised file" using a fault-injecting test hook in `open` that fails after the first DDL statement
- [ ] 4.7 Implement `storage/stats.rs` per design D-18: commit counter, `PRAGMA optimize` after every `optimize_every`-th commit and after a commit that inserted at least `optimize_every` statements (after `COMMIT`, errors ignored, dry runs and speculation not counted), and `analyze()` running a full `ANALYZE`; add `// @lat: [[query#Physical Planning#Join Ordering]]` on the upkeep function
- [ ] 4.8 Tests for `storage-format` "Planner statistics are part of the store" (statistics after open + load without `optimize()`, periodic optimize with `optimize_every = 10`, stale statistics never change results, full analysis), "Format 1 reserves names for later milestones" and the "Live indexes keep t_ret in their key" scenario

## 5. Term dictionary

- [ ] 5.1 Implement `term.rs` `TermDict` writer: `lookup_or_insert(tag, lex, dt, lang, num)` with the `IS`-based lookup of design D-5, allocation from `next_term`, committed cache + per-transaction overlay with `commit()` / `rollback()`
- [ ] 5.2 Implement `TermReader`: lookup-only encode (`Option<ObjectId>`), decode by id with a shared `Mutex<LruCache>`; `InvalidTerm` for unknown ids, `Unsupported` for tag 15 `SEALED`
- [ ] 5.3 Tests for `object-encoding` "Term dictionary deduplication and immutability" and "Lookup without insertion on the read path", including the NULL `dt`/`lang` dedupe scenario, plus "fail after interning then intern again" cache-consistency test

## 6. Transaction engine core

- [ ] 6.1 Implement `clock.rs` (`Clock`, `SystemClock`, `ManualClock` with `set()`/`advance()`)
- [ ] 6.2 Implement `engine/mod.rs` `WriteCtx`: `BEGIN IMMEDIATE`, counter load, `t = last_t + 1`, `instant = max(now, last_instant + 1)`, `INSERT INTO tx` at begin, id allocation (`alloc_stmt`, `alloc_node`, `alloc_bnode`), commit (write counters, COMMIT, merge overlays) and rollback paths
- [ ] 6.3 Implement `report.rs` types (`TxReport`, `Asserted`, `RetKind`, `TxOptions` with defaults, `Valid`, `AssertOpts`, `OnExisting`, `Patch`, `Triple`, `Event`, `Op`) and the report-building rules of design D-7 (`existing` dedupe/exclusion)
- [ ] 6.4 Implement `engine/reserved.rs` allow-list check and position validation (`InvalidTerm`) shared by every write path
- [ ] 6.5 Tests for `transactions` "Gap-free transaction numbers", "Every committed transaction is recorded" and "Transactions are nodes with metadata triples"
- [ ] 6.6 Tests for `transactions` "Strictly increasing transaction instants" with `ManualClock` (backwards, standing still, normal); add `// @lat: [[tests#Time Travel#Instants Are Monotonic]]` on the backwards-clock test (it also asserts `as_of(Instant)` resolution once task 11.2 lands)
- [ ] 6.7 Tests for `transactions` "Atomic failure leaves no trace" (snapshot of all five tables before/after), including caller-aborted body and "ids from a failed transaction may be reissued"
- [ ] 6.8 Tests for `sql-executor` "A failed transaction leaves no trace on any host" (failed body on both hosts, injected busy error on `MinimalHost`) and "The core runs on the required operations alone" (the whole suite passes on `MinimalHost`; the SQL it recorded uses no user function, virtual table or FTS5)

## 7. Statement lifecycle operations

- [ ] 7.1 Implement `assert_with` / `assert` per design D-8 steps 1–4 and 7 (encoding, validation, `InvalidInterval`, idempotency query with the overlap formula, smallest-eid match, `SelfReference`, insert), without schema steps yet
- [ ] 7.2 Implement `create`, `meta`, `new_node`, `new_bnode`, and `confirm` (via engine-internal assert of `sys:confirmedBy`, `NotLive` check) and `OnExisting::Confirm`
- [ ] 7.3 Tests for `statement-lifecycle` "Assert is idempotent over overlapping valid time" (all overlap-boundary scenarios); add `// @lat: [[tests#Operations#Assert Is Idempotent]]` on the same-fact-twice test and `// @lat: [[tests#Operations#Non-Overlapping Episodes Coexist]]` on the episodes test
- [ ] 7.4 Tests for "Create always inserts"; add `// @lat: [[tests#Operations#Create Makes Parallel Edges]]` on the parallel-edges test
- [ ] 7.5 Tests for "Statement content and positions", "Statement references are not checked for liveness", "Valid time is a half-open interval" (empty/reversed/one-sided), "Assert can confirm an existing match", "Confirm records corroboration", "New nodes", "No direct self-reference", "Reserved sys namespace" and "Eids are never reused"

## 8. Retraction and cascade

- [ ] 8.1 Implement `engine/cascade.rs` `cascade_set` (BFS over `live_spo`/`live_osp`, visited set, per-root limit) and `retract_root` with kind rules of design D-9; add `// @lat: [[time-model#Cascade]]` on `cascade_set`
- [ ] 8.2 Implement `retract` (returns false for retracted/unknown eids) and `retract_matching` (snapshot, ascending eid, returns all matches)
- [ ] 8.3 Tests for `statement-lifecycle` "Retract sets the retraction exactly once" and "Retract by pattern"; add `// @lat: [[tests#Operations#Retract Is Once]]` on the second-retraction test
- [ ] 8.4 Tests for `retraction-cascade` "Cascade over subject and object positions"; add `// @lat: [[tests#Cascade#Cascades Subject And Object]]` on the annotation-and-reference test
- [ ] 8.5 Tests for "Cascades terminate on cycles" (forward-reference cycle, diamond); add `// @lat: [[tests#Cascade#Cascade Terminates On Cycles]]` on the two-statement test
- [ ] 8.6 Tests for "Cascade size limit" (exceeded, exactly at limit, per root); add `// @lat: [[tests#Cascade#Cascade Limit Aborts]]` on the exceeded test (asserts the database is unchanged)
- [ ] 8.7 Tests for "Retraction kinds" and "Retracted structures remain visible in the past"

## 9. Predicate schema

- [ ] 9.1 Implement `engine/schema.rs` `PredicateSchema` loading, per-transaction cache and invalidation on flag writes/retractions; built-in validation of flag values and flag subjects; implicit cardinality-one of flag predicates
- [ ] 9.2 Implement the value-type matcher (tag IRIs, datatype IRIs → tag sets plus `TYPED` with matching `dt`) and wire `ValueTypeMismatch` into assert/create (design D-8 step 3)
- [ ] 9.3 Wire the unique check (D-8 step 5) and cardinality-one replacement via `retract_root(.., Cardinality)` (D-8 step 6) into assert and create, in the order value type → unique → cardinality
- [ ] 9.4 Implement schema-change validation queries for `one`, `unique` and `valueType` returning sorted `SchemaConflict { violating }` (design D-10), run before inserting a flag
- [ ] 9.5 Implement `upsert` (`NotUniquePredicate`, live lookup on `live_pos`, else `new_node` + assert)
- [ ] 9.6 Tests for `predicate-schema` "Schema flags are versioned statements" and "Schema flag values are validated"
- [ ] 9.7 Tests for "Cardinality one replaces overlapping objects"; add `// @lat: [[tests#Operations#Cardinality One Does Not Replay]]` on the annotations test
- [ ] 9.8 Tests for "Unique predicates allow one live subject per value"; add `// @lat: [[tests#Operations#Unique Rejects Second Subject]]` on the second-subject test (asserts no trace)
- [ ] 9.9 Tests for "Upsert on a unique predicate"; add `// @lat: [[tests#Operations#Upsert Returns Existing Node]]` on the existing-subject/new-node test
- [ ] 9.10 Tests for "Value type constrains objects", "Edge flag is stored", "Schema changes violated by live data are rejected" and "Order of schema checks"
- [ ] 9.11 Reject `sys:sensitive` with `Unsupported` before the allow-list check (design D-11); tests for `predicate-schema` "Sensitive flag is reserved" and `object-encoding` "Reserved SEALED tag is rejected"; add `// @lat: [[tests#ObjectId#Reserved Tag Is Rejected]]` on one test that covers both

## 10. Supersede

- [ ] 10.1 Implement `engine/supersede.rs` per design D-12 (NotLive, patch application and `InvalidPatch` rules, cascade set, σ allocation in BFS order, retract with `Supersede`, root schema checks, replay with σ rewrite, `sys:supersedes` link, report pairs); add `// @lat: [[time-model#Operations#Supersede]]` on the function
- [ ] 10.2 Implement `Patch::from_fields` in the facade for bindings (rejects `s`/`p` with `InvalidPatch`)
- [ ] 10.3 Tests for `supersede` "Supersede retracts and replays the cascade set" (annotation, reference, deep layer, cycles); add `// @lat: [[tests#Operations#Supersede Replays Layers]]` on the replay test
- [ ] 10.4 Tests for "Supersede links the new root to the old one" (including the chain), "Patch rules" (all six scenarios) and "Only live statements can be superseded"
- [ ] 10.5 Tests for "Supersede is bounded and schema-checked" and "Supersede report and event log"

## 11. Views and reads

- [ ] 11.1 Implement `view.rs` `ViewSpec`, `TxSel`, `TimeRef`, `ValidSel` and `scan_predicates()` emitting exactly the shapes of design D-14 (verbatim `t_ret IS NULL`, instant CTE); add `// @lat: [[query#Views and Scans]]` on `scan_predicates`
- [ ] 11.2 Implement `read.rs` `triples()` (full rows, eid order, lookup-only constants, `AsOf` masking of `t_ret`/`ret_kind`) and `events_since()` with the ordering of design D-14
- [ ] 11.3 Tests for `temporal-views` "A view is a transaction-time selector plus a valid-time selector", "Now view", "As-of view by transaction", "As-of view by instant", "History view", "Valid-at filter" and "Triple-pattern lookup"; extend the backwards-clock test from 6.6 with instant resolution
- [ ] 11.4 Tests for "Event log since a transaction"
- [ ] 11.5 Test "View scans use covering indexes": run `EXPLAIN QUERY PLAN` on the now, as-of, valid-at and object-bound shapes built by `scan_predicates` projecting `(s|p|o, eid)` and assert a covering `live_*`, `hist_*` or `valid_p` index; add `// @lat: [[tests#Storage Invariants#Views Use Covering Indexes]]`

## 12. Volatile state

- [ ] 12.1 Implement `engine/volatile.rs` `set_volatile` (upsert, `updated_at = instant`, IRI key check) and `clear_volatile`
- [ ] 12.2 Implement `read.rs` `values(s, key)` resolution (triple objects first; volatile only under `Now`)
- [ ] 12.3 Tests for all `volatile-state` requirements (overwrite, failed tx discards, clear twice, not a triple, not in events, now-only visibility, statement wins)

## 13. Speculation and dry runs

- [ ] 13.1 Implement the shared savepoint mechanism of design D-13: `BEGIN IMMEDIATE`, `SAVEPOINT spec`, run ops, `ROLLBACK TO spec; RELEASE spec`, write advanced `next_*` counters only if changed, `COMMIT`, drop overlays; burn on success and failure
- [ ] 13.2 Implement `TxOptions::dry_run` on top of 13.1, returning the in-savepoint report with the would-be `t`/`instant`
- [ ] 13.3 Implement the writer-connection `View` (Now + optional `valid_at`) used inside `with`, bypassing the shared reader LRU
- [ ] 13.4 Tests for `transactions` "Transaction options" (defaults, dry-run report and `t` reuse) and `retraction-cascade` "Dry-run preview of a cascade"; add `// @lat: [[tests#Cascade#Dry Run Reports Without Commit]]` on the preview test (asserts only `meta` id counters changed)
- [ ] 13.5 Tests for `speculative-transactions` "Speculation runs the full engine and exposes uncommitted state" and "Speculation leaves no trace"; add `// @lat: [[tests#Storage Invariants#Speculation Leaves No Trace]]` on the tables-unchanged test (also asserts counters advanced past every speculative id)
- [ ] 13.6 Tests for "Ids allocated during speculation are burned" (statement, node, term ids, dry run, reopen)

## 14. Facade: Db, readers and concurrency

- [ ] 14.1 Implement `tiramemsu::db` `OpenOptions` (readers 4, clock, busy_timeout 5 s, term_cache_capacity 16 384, optimize_every 1000), `Db::open` (with `RusqliteHost`) and `Db::open_with_host` delegating to `storage::open`, `Db::optimize()` and `Db::capabilities()`
- [ ] 14.2 Implement `pool.rs` `ReaderPool` (read-only executors, only when the host declares `reader_pool`, `Mutex<Vec<_>>` + `Condvar`, each read wrapped in `begin_read`…`commit` for one snapshot); without `reader_pool`, reads run on the writer under its mutex
- [ ] 14.3 Implement the writer `Mutex<Box<dyn Executor>>` with the thread-local re-entrancy guard (`Error::Reentrant`), `Db::transact`, `Db::with`, `Db::now/as_of/history`, `Db::events_since`
- [ ] 14.4 Implement facade `Tx<'a>` and `View` wrappers with the signatures of design D-2 and `IntoObject` for `ObjectId`, `Eid`, `TxId`, `Value`, `&Value`; rustdoc on every public item (including "ids seen in a failed body may be reissued" and "as-of rows hide later retractions")
- [ ] 14.5 Tests for `storage-format` "Opening an existing database preserves its contents" (reopen numbering and history) and "One writer connection and a pool of readers" (reads during a long write, consistent snapshot)
- [ ] 14.6 Tests for `transactions` "Single writer serialisation" (8 threads × 100 transactions, concurrent upsert of a unique value, two handles on one file) and "Re-entrant writes are rejected"; `speculative-transactions` "Speculation holds the single writer and starts from now"

## 15. Never-forget enforcement

- [ ] 15.1 Tests with a raw `rusqlite` connection (opened without the engine) for every scenario of `never-forget` "Statements are never deleted", "A statement is retracted at most once and its content never changes", "Terms are never deleted or changed" and "Transactions are never deleted or changed"; add `// @lat: [[tests#Storage Invariants#Triggers Block Deletion]]` on the triple-delete test
- [ ] 15.2 Tests for "The engine never issues deletions or rewrites" (savepoint rollback succeeds; the grep step of task 1.5 is referenced from the test file comment), "Erasure never deletes rows" and "Volatile and metadata tables are outside the invariant"

## 16. Property tests over operation sequences

- [ ] 16.1 Build a `proptest` strategy that generates operation sequences (assert with random valid intervals, create, retract, retract_matching, supersede with random patches, cardinality-one asserts on a `sys:one` predicate, annotations on existing eids) over a small fixed vocabulary, grouped into random transactions
- [ ] 16.2 Property: for every `t`, `as_of(Tx(t)).triples(None, None, None)` equals the state replayed from `events_since(0)` up to `t`; add `// @lat: [[tests#Time Travel#AsOf Equals Replay]]`
- [ ] 16.3 Property: as-of results recorded after each commit are identical after further random transactions; add `// @lat: [[tests#Time Travel#Historical Reads Are Stable]]`
- [ ] 16.4 Property: random operation sequences never trigger an invariant-trigger abort and never produce duplicate `(tag, lex, dt, lang)` terms (no lat.md leaf; spec `never-forget` "Engine operations under the triggers")

## 17. Benchmarks skeleton

- [ ] 17.1 Add a `criterion` bench crate or `benches/` in `tiramemsu` with the churn benchmark (N = 1, 10, 100, 1000 updates per key; now vs as-of lookup throughput) and supersede cost vs cascade-set size from `lat.md/roadmap#Benchmarks`, plus executor-trait overhead against a direct `rusqlite` loop (design D-17 risk); no targets asserted, results printed only

## 18. lat.md anchoring and wrap-up

- [ ] 18.1 Add `// @lat:` code refs from implementation to the design sections they implement where not already added (e.g. `[[time-model#Operations#Assert]]`, `[[time-model#Operations#Cardinality One]]`, `[[time-model#Operations#Unique Upsert]]`, `[[time-model#Speculative Transactions]]`, `[[storage#Volatile Table]]`, `[[data-model#ObjectId]]`, `[[data-model#ObjectId#Canonical Encoding]]`, `[[storage#Format Versioning]]`)
- [ ] 18.2 Update `lat.md/` for the behaviour fixed in this change (the "chosen during spec writing" decisions of design.md, e.g. extra error variants in `api#Errors`, `View::values`, `Tx::clear_volatile`/`new_bnode`) and add test-spec leaves in `lat.md/tests.md` for new core invariants worth tracking; run `lat check` until it passes
- [ ] 18.3 Once every non-Query leaf in `lat.md/tests.md` carries a `// @lat:` ref, add the `require-code-mention: true` frontmatter to `lat.md/tests.md` and run `lat check`; if it fails only on the `Query` leaves owned by M1–M3, move them to a separate test-spec file or leave the flag off and record it as a follow-up for the last of those changes
- [ ] 18.4 Run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, and `openspec validate add-core-store --strict`; all pass
