# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Entries before 0.4.0 were reconstructed from the git history and the archived
OpenSpec changes in `openspec/changes/archive/`.

## [Unreleased]

## [0.5.0] - 2026-10-11

Vector and hybrid recall, and the fixes of the 2026-10-10 review (`docs/project-review.md`).

**Upgrade notes**

- **Storage format 5.** Opening a format-4 file adds the empty `embedding` table and its never-forget triggers, in one transaction; no graph row is touched. Older builds refuse a format-5 file with `FormatVersion`.
- New error variant `Error::EmbeddingConflict` (bridge code `EmbeddingConflict`); `Error` is `#[non_exhaustive]`.
- Five more MCP tools are offered (`define_embedding_model`, `set_embedding`, `vector_search`, `recall`, `unembedded`); clients that pin the tool list should refresh it.

### Fixed

- Single-table lookups (`View::triples`, write-path existence and cardinality checks, cascade walks, term dictionary lookups) no longer re-prepare on every call once `ANALYZE` has written STAT4 samples. A point lookup after churn drops from 16.6 µs to 2.7 µs, and SPARQL point and two-hop queries at 10⁵ statements run 2–2.7× faster (55 → 20 µs, 92 → 45 µs).
- `supersede` and retraction cascades load their statements in batched queries instead of one query per member (a 1 000-statement supersede: 83 → 68 ms).
- The writer's term cache is scan-resistant: a bulk chunk of new terms no longer evicts the warm set.
- A budgeted call waiting for the writer sleeps until the writer is released instead of polling every millisecond.
- LFTJ charges its materialised inputs to the operation's byte budget, so `max_bytes` bounds its memory.
- JSON bridge: `newNode` with `as` binds the node, so `{"ref": …}` resolves to it in later ops. Python `TxBuilder.new_node()` now returns a `Ref`, and Node `Tx.newNode()` returns a `Ref` that actually resolves.
- The README quick start compiles as shown.

### Added

- **Vector recall** (`add-vector-recall`): `Tx::define_embedding_model` (dimensions, cosine/dot/L2, f32/f16) and `Tx::set_embedding` store caller-supplied vectors in the write transaction; `View::vector_search` runs an exact, view-aware search (as-of views see only embeddings stored by then) with an optional sign-bit prefilter; `View::unembedded` lists the backlog. Also SPARQL `tm:vectorMatch` / `tm:vectorModel` / `tm:vectorSimilarity` / `tm:vectorRank` / `tm:vectorLimit`, Cypher `CALL tiramemsu.vector.search(...)`, the `tm_vector` table function, bridge ops `defineEmbeddingModel`, `setEmbedding`, `vectorSearch`, `unembedded`, the MCP tools above, and Python and Node methods. No SQLite extension is needed, so it works on WASM too.
- **Hybrid recall** (`add-hybrid-recall`): `View::recall` fuses text and vector recall by reciprocal rank (k = 60) and can expand each hit to its layers and subject facts; SPARQL `tm:recall*` patterns, Cypher `CALL tiramemsu.recall({...})`, the `tm_recall` table function, bridge `recall`, MCP `recall`, Python `View.recall`, Node `View.recall`.
- Vector recall predicate filter in the query languages: Cypher `predicates` option and SPARQL `tm:vectorPredicate`. MCP `unembedded` tool, `confidence` on `vector_search` and `depth` on `recall`.
- `vector_footprint` example measuring bytes per embedding; vector and hybrid recall benchmarks; prefilter quality measured on GloVe-100 (see `crates/tiramemsu/benches/README.md`).
- `vector` benchmark (`cargo bench -p tiramemsu --bench vector`).
- `docs/vector-architecture.md` and the article `site/articles/vector-memory.html`.
- `hotspots` benchmark (`cargo bench -p tiramemsu --bench hotspots`): term cache after a burst, contended budgeted writes, instant- versus transaction-based as-of.
- The Lean model of the store contract moved to `lean/` as a Lake project, built in CI.
- OpenSpec proposals for the next releases under `openspec/changes/`. `add-libsql-host` is deferred: vector recall is host-neutral instead.

## [0.4.0] - 2026-10-04

Conformance repairs for the four integrity gaps found by the paper review
(`lat.md/overview.md#Formal Model`). Published on crates.io (`tiramemsu`,
`tiramemsu-mcp`) and PyPI (`tiramemsu`); `@tiramemsu/node` is not yet on npm.

### Upgrade notes

- **Storage format 4.** Opening a format-3 file migrates it forward automatically, in one transaction. Once migrated, 0.3.0 refuses the file with `FormatVersion`.
- **Writes are stricter.** `assert` and `create` (and the SPARQL and Cypher writes built on them) now fail on a statement endpoint that is retracted or was never allocated. Before, they accepted it.
- **Raw SQL writers** must follow the engine protocol: insert the `tx` row first, and use dates above the committed `meta.last_t`.

### Added

- Transaction-date guard triggers `triple_add_date` and `triple_ret_date` (storage format 4). Statement dates must name the latest transaction, strictly above the committed `meta.last_t`.
- Regression tests for the conformance repairs (`crates/tm-core/tests/conformance.rs`), run on both SQLite hosts.

### Changed

- Statement endpoints in subject or object position must be live: retracted or never-allocated eids are rejected, and the transaction leaves no changes.
- `supersede` keeps a closed retention set: it drops foreign memberships and their layers before copying, so it no longer replays layers onto eids that were never inserted.

### Fixed

- Raw SQL can no longer backdate statement assertions or retractions, or write dates outside an active transaction.

## [0.3.0] - 2026-10-04

Ten features added since 0.2.0. Published on crates.io (including the new
`tiramemsu-mcp` crate and binary) and PyPI.

### Upgrade notes

- **Storage format 1 → 3.** Format 2 adds text-index bookkeeping rows to `meta`. Format 3 adds the derived tables `saved_answer` and `saved_answer_dep`. Older files migrate forward on open, and builds before 0.3.0 refuse the migrated file with `FormatVersion`.
- **Cargo features changed.** `default-features = false` now builds the core tier, with no query engine and no parsers. Dependants that turn off default features and use SPARQL, Cypher or paths must add `features = ["sparql", "cypher"]`, or the subset they use.

### Added

- Query budgets: `QueryBudget`, `View::with_budget`, `Db::transact_budgeted`, `Db::cypher_write_budgeted`, `CancelToken`, `OpenOptions::reader_timeout`. New errors `Cancelled`, `DeadlineExceeded`, `PoolTimeout`, `ResultLimitExceeded`.
- Bulk import sessions: `Db::bulk_import`, `BulkImport::{chunk, progress, finish, cancel}`, `ImportProgress`, `ImportSummary`, `Db::statistics_due`. New error `ImportInProgress`. Planner statistics are refreshed once at the end of the session.
- Text recall: `View::text_search` over an FTS5 index with evidence ranking, `Db::enable_text_index`, `Db::rebuild_text_index`, `OpenOptions::text_index`. New error `TextIndexUnavailable`.
- Text recall from the query languages: SPARQL `tm:textMatch` (plus `tm:textScore`, `tm:textRank`, `tm:textConfidence`) and Cypher `CALL tiramemsu.text.search(...)`.
- `tiramemsu-mcp`: a local stdio MCP memory server. Its tools are `assert`, `confirm`, `supersede`, `query`, `dependents`, `export_bundle`, `import_bundle` and `text_search`, with a read-only mode and query budgets.
- Provenance coverage reporting: `SparqlOptions::query_only`, `Solutions::provenance_gaps`, `provenance_complete()`.
- Saved answers: `Db::save_answer`, `saved_answers`, `check_saved_answers`, `refresh_answer`, `delete_saved_answer`. Stale answers are marked conservatively, driven by events.
- Temporal path syntax: SPARQL `SERVICE <urn:tiramemsu:tm:timeRespecting>` with `tm:arrival`, and Cypher `MATCH TIME RESPECTING [AFTER t] [ARRIVAL AS name]`. Path results report whether they are complete (`PathCompleteness`).
- Opt-in leapfrog triejoin operator for cyclic joins (`OpenOptions::planner.lftj`, `tm_lftj`), with the routing reasons shown in explain output.
- Conflict review: `View::conflicts` with attributed evidence, and `Db::preview_bundle` to preview a bundle import without committing it.
- Cargo features `exec`, `sparql` and `cypher` on `tiramemsu` (default `sparql` + `cypher`). RDF terms and the N-Triples writer moved to `tm_core::rdf`.
- WebAssembly host: crate `tm-wasm` (memory and OPFS storage, journal-mode verification, capability probing, file export and import) and the `tiramemsu-wasm` Web Worker binding.
- Node.js, Python and JSON-bridge operations for all of the above. `tiramemsu-json` can now be published.

### Fixed

- Bundle property test: correction objects are now unique per transaction, and the seed that failed is kept as a regression test.

## [0.2.0] - 2026-10-03

### Upgrade notes

- **Bounded id counters.** `NODE`, `BNODE`, `STMT` and `TX` ids now reserve 12 origin bits. Counters stop at 2⁴⁸ − 1 and then fail with the new `IdSpaceExhausted` error. Existing ids do not change, and the storage format stays at 1.
- Input ids with a non-zero origin are rejected: in skolem IRIs, in `Value::Stmt` and on bundle import.

### Added

- Statement dependents: `View::dependents(eid)` returns the statements a cascade would take, under any view.
- Fact bundles: `View::bundle`, `Tx::import_bundle`, `BundleFormat::to_json` / `from_json` (`tiramemsu-bundle/1`) and `to_ntriples`.
- SPARQL query provenance: `View::sparql_with` with `SparqlOptions { provenance: true }` and `Solutions::provenance(row)`.
- Statement instants: the virtual predicates `tm:addedAt` / `tm:retractedAt`, available in Cypher as `r.addedAt` / `r.retractedAt`.
- Typed layers: schema flag `sys:subjectType` and new error `SubjectTypeMismatch`.
- Graph-scoped paths: property paths inside `GRAPH`, `GRAPH ?g` and `FROM`. Adds `View::path_with` with `PathArgs { graphs, .. }` and a `graphs` argument to `tm_path`.
- Time-respecting (earliest-arrival) paths: `PathArgs.time_respecting`, an `arrival` field on path rows, and `timeRespecting` in the `tm_path` view.
- Statement graphs: a statement eid can name a graph. `supersede` carries a corrected edge's memberships over to the new eid.
- Node.js and Python wrappers for provenance, path `graphs` and `arrival`, `dependents`, `bundle` and `importBundle`.

### Fixed

- `Store::read`, `Store::transact` and speculation recover from a panicking callback: they roll back SQLite and the term dictionary, then re-raise the panic. Speculation also burns the ids it allocated.
- The reader pool gets its executor back after a caught panic, and rolls back a failed read commit before the executor is reused.
- Cypher parses `INF`, `INFINITY` and `NAN` as float literals instead of failing with "invalid float".

## [0.1.0] - 2026-09-30

First release.

### Added

- Core store (`tm-core`, `tm-rusqlite`, facade `tiramemsu`): 64-bit ObjectIds, a term dictionary, a single-writer bitemporal transaction engine, and storage format 1 in WAL mode with never-forget triggers.
- Write operations: `assert`, `create`, `retract` / `retract_matching` with cascade, `supersede`, `confirm`, `upsert`, speculative transactions (`Db::with`) and `dry_run`.
- Views: now, as-of (by transaction or instant), valid-at and history, plus `events_since` and a volatile side table.
- Optional predicate schema: `sys:cardinality`, `sys:unique`, `sys:valueType`, `sys:isEdge`.
- Query IR and SQL planner (`tm-ir`, `tm-exec`): `View::execute_ir`, `View::explain_ir`.
- SPARQL 1.1 / 1.2 front end (`tm-sparql`): `View::sparql`, covering queries, updates, triple terms, reifiers and annotations, with `tm:` time IRIs.
- Cypher front end (`tm-cypher`): `View::cypher`, `Db::cypher_write`, temporal `USE` clauses and statements readable as both relationships and nodes.
- Native path engine: `View::path` and the `tm_path` table function. SPARQL property paths and Cypher variable-length / `shortestPath` patterns run on it.
- Named graphs: `Tx::add_to_graph`, `remove_from_graph`, `clear_graph`, `View::graphs`, `View::graph_members`. SPARQL gains `GRAPH`, `FROM NAMED` and graph management updates.
- JSON bridge (`tiramemsu-json`), Node.js bindings (`@tiramemsu/node`) and Python bindings (`tiramemsu`), with release workflows for PyPI wheels and npm.

[Unreleased]: https://github.com/Volland/tiramemsu/compare/v0.5.0...HEAD
[0.5.0]: https://github.com/Volland/tiramemsu/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Volland/tiramemsu/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Volland/tiramemsu/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Volland/tiramemsu/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/Volland/tiramemsu/releases/tag/v0.1.0
