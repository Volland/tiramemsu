## Why

Tiramemsu's whole design (addressable triples, Datomic-style transaction time plus valid time, never-forget history, SPARQL and Cypher over one store) rests on a storage and transaction core that does not exist yet. Milestone M0 (`lat.md/roadmap#Milestones`) builds that core first, because every later change (IR and SQL planner, SPARQL, Cypher, path engine) depends on its ObjectIds, its SQLite schema, its write operations and its time views.

## What Changes

- New Cargo workspace with three crates: `tm-core` (ObjectId codec, term dictionary, SQLite schema, migrations and invariant triggers, transaction engine, views, event log, volatile table, predicate schema, and the `Executor` trait), `tm-rusqlite` (the first executor host: `rusqlite` with bundled SQLite) and the `tiramemsu` facade (`Db`, `View`, `Tx`, `TxReport`, errors). No query language is included; reads go through a `triples(s?, p?, o?)` pattern lookup on a `View`.
- A synchronous executor trait (`lat.md/architecture#Executor`): `tm-core` reaches SQLite only through prepared statements, interactive transactions, savepoints and read snapshots, and reads the host's declared capabilities (`reader_pool`, `functions`, `vtab`, `stat4`, `fts5`). `tm-core` does not depend on `rusqlite`.
- One SQLite file per database in WAL mode with the STRICT format-version-1 schema of `lat.md/storage#Schema`, `meta` id counters, forward migrations, and a `FormatVersion` error for files written by a newer format. Format 1 reserves tag 15 `SEALED`, `sys:sensitive` and the table `seal_key` for crypto-shredding (M6), and the tables `term_fts` and `vec_*` for retrieval (M7).
- Planner statistics kept by the store (`lat.md/query#Physical Planning#Join Ordering`): `PRAGMA optimize=0x10002` at open, `PRAGMA optimize` after bulk loads and every `OpenOptions.optimize_every` commits, and a full `ANALYZE` through `Db::optimize()`.
- 64-bit low-tagged ObjectIds with canonical encoding (date-times keep their timezone offset inside the inline payload), a deduplicating term dictionary, and skolem IRIs for anonymous and blank nodes.
- A single-writer transaction engine: gap-free `t`, strictly increasing `instant`, transaction metadata triples, `TxReport`, `TxOptions` (`dry_run`, `max_cascade`), atomic failure.
- Statement operations: idempotent `assert` over overlapping valid time, `create` for parallel statements, `retract` and `retract_matching` with recursive subject/object cascade, `supersede` (cascade and replay under a substitution map), `confirm`, `upsert`, `new_node`, `meta`.
- An optional predicate schema stored as `sys:` triples: `sys:cardinality`, `sys:unique`, `sys:valueType`, `sys:isEdge`, with rejection of schema changes that live data violates.
- Temporal views over the `triples()` lookup: now, as-of (by transaction or instant), valid-at, history, plus `events_since(t)` over the `event` view.
- Never-forget enforcement in the file itself through SQLite triggers that block DELETE and content UPDATE.
- Speculative transactions (`Db::with`) and `dry_run` on a SAVEPOINT that roll back and burn every id they allocated.
- A `volatile` side table for high-churn, non-historical state, visible only in the now view.

## Capabilities

### New Capabilities

- `sql-executor`: the synchronous `Executor` trait, its required operations, host-declared capabilities, the `tm-core` tier that runs on the required set alone, and the `rusqlite` host.
- `storage-format`: creating and opening the database file, WAL mode, the STRICT format-1 schema and its reserved names, `meta` counters, planner statistics, format-version checks and forward migration.
- `object-encoding`: the 64-bit ObjectId with a 4-bit low tag (tag 15 `SEALED` reserved), canonical encoding rules including date-times with their offset, round trip, signed order within a tag, the term dictionary, and skolem IRIs for NODE and BNODE ids.
- `transactions`: transaction numbering and instants, transaction metadata, `TxReport`, `TxOptions`, atomic failure and single-writer serialisation.
- `statement-lifecycle`: `assert`, `create`, `retract`, `retract_matching` and `confirm`; valid-time interval semantics and validation; self-reference and reserved-namespace rules.
- `retraction-cascade`: recursive retraction over subject and object positions, cycle handling, the cascade limit, dry-run preview and `ret_kind` values.
- `supersede`: correcting a statement by cascade-and-replay with a substitution map, patch rules, the `sys:supersedes` link and the `NotLive` error.
- `predicate-schema`: `sys:cardinality`, `sys:unique` with `upsert`, `sys:valueType`, `sys:isEdge`, schema versioning and `SchemaConflict`; `sys:sensitive` reserved for M6.
- `temporal-views`: now, as-of, valid-at and history semantics of `triples()`, stability of historical reads, `events_since`, and the as-of-equals-replay property.
- `never-forget`: database-level triggers that forbid deleting or rewriting triples, terms and transactions, even from raw SQLite connections.
- `speculative-transactions`: `Db::with` on a SAVEPOINT, reads of uncommitted state, rollback without trace, and burned ids.
- `volatile-state`: the `volatile` side table, its upsert semantics and its visibility rules.

### Modified Capabilities

None. This is the first change; `openspec/specs/` is empty.

## Impact

- **New code:** `Cargo.toml` workspace, `crates/tm-core`, `crates/tm-rusqlite`, `crates/tiramemsu`, CI configuration (fmt, clippy, tests).
- **Dependencies:** `rusqlite` with the `bundled` feature (a pinned SQLite with STRICT tables, partial indexes, STAT4 and FTS5; the plans in `lat.md/storage#Query Shapes` were verified on 3.53), used only by `tm-rusqlite` (and as a dev-dependency for tests); `thiserror`; dev-dependencies `proptest` and `tempfile`.
- **Public API:** the `tiramemsu` facade surface of `lat.md/api#Rust Surface`, minus `sparql`, `cypher` and `path` (added by later changes), plus `Db::optimize()` and `OpenOptions.optimize_every`. The error enum of `lat.md/api#Errors` minus `Parse` and the query-only variants, plus the few variants recorded in `design.md`. `Unsupported { feature }` is included, for the features reserved for M6.
- **On-disk format:** format version 1 is fixed by this change. Later changes may only add migrations that respect never-forget.
- **Downstream:** `add-query-ir-and-sql-planner` builds on the ObjectId codec, the term dictionary, the view descriptor and its single time-predicate function, and the reader/writer connection split defined here.
- **Documentation:** `lat.md/` gains `@lat` code references from the new tests and code; no design text changes.
