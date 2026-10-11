## Status

Implemented (unreleased, after 0.4.0; storage format 5), phase 1 and phase 2. Redesigned 2026-10-11 (host-neutral, exact search); does not depend on `add-libsql-host`. Phase 1 = groups 1 to 7 (Rust, bridge, MCP, bindings); Phase 2 = group 8 (SPARQL and Cypher). Unchecked items below are open follow-ups.

## 1. Contract

- [x] 1.1 Finalise names: `EmbeddingModel`, `Metric::{Cosine, Dot, L2}`, `VectorEncoding::{F32, F16}`, `VectorQuery`, `VectorHit`, `VectorResult`, `Error::EmbeddingConflict`, ranking policy id `tiramemsu-vector-rank/1`.
- [x] 1.2 Share text recall's evidence (confidence, confirmations, authors, `t_add`, `added_at`) as one function used by both recalls.

## 2. Vocabulary and definitions

- [x] 2.1 Add `sys:dimensions`, `sys:metric`, `sys:vectorEncoding` to `vocab.rs`; engine-owned (users writing them get `ReservedNamespace`); validate types and values (dimensions 1 to 16 384).
- [x] 2.2 `Tx::define_embedding_model` returning `true` (new), `false` (identical), `InvalidQuery` (different); `InvalidTerm` for a non-IRI model.
- [x] 2.3 Read a model definition in a view.

## 3. Storage format 5

- [x] 3.1 `ddl_embedding.sql` (`embedding` table, `embedding_model_eid`, covering index `embedding_bits` on `(model, eid, bits)`, `embedding_no_delete`, `embedding_no_update`); `v4_to_v5` in `MIGRATIONS`; `FORMAT_VERSION = 5`.
- [x] 3.2 Add `embedding` to the expected-schema and foreign-file checks.

## 4. Embeddings

- [x] 4.1 Encoding: cosine normalisation (zero vector rejected), little-endian f32/f16, sign bits (`ceil(dim/8)` bytes).
- [x] 4.2 `Tx::set_embedding`: live check (`NotLive`), model and dimension and finiteness checks (`InvalidQuery`), idempotent `false`, `EmbeddingConflict`; insert with `t`.
- [x] 4.3 `View::unembedded(model, limit)`, honouring the as-of embedding-time rule.

## 5. Recall

- [x] 5.1 `tm_core::vector::search`: SQL over `embedding ⨝ triple` with `scan_predicates`, `e.t <= t` for as-of views, predicate filter and graph-membership `EXISTS`; scan reads only `(eid, blob)` with no `ORDER BY`; Rust similarity (cosine, dot, negative l2) and bounded top-k; f16 decoded through a 65 536-entry table.
- [x] 5.2 Statement details and evidence for kept hits only (including every row tied at the cut); ranking `tiramemsu-vector-rank/1`; `rank`, `limit`; `candidates`, `exact`.
- [x] 5.3 Binary prefilter (`prefilter: Some(n)`): Hamming on `bits` read from the covering index, exact re-rank on `vec`, `exact = false`.
- [x] 5.4 Budget: per-row polling; hits charged as rows and bytes.
- [x] 5.5 Facade `View::vector_search`, `View::unembedded`, `Tx::define_embedding_model`, `Tx::set_embedding`.

## 6. Bridge, MCP and bindings

- [x] 6.1 JSON bridge transaction ops `defineEmbeddingModel`, `setEmbedding` (eid or ref) and read ops `vectorSearch`, `unembedded`; error code `EmbeddingConflict`.
- [x] 6.2 MCP tools `define_embedding_model`, `set_embedding` (write, hidden in read-only mode), `vector_search` (read, bounded `limit`).
- [x] 6.3 Python `tx.define_embedding_model(model, dimensions, metric=, encoding=)`, `tx.set_embedding(eid, model, vector)`, `view.vector_search(model, vector, limit=, graphs=, predicates=, prefilter=, confidence=)` (iterable `VectorResult`), `view.unembedded(model, limit=None)`; Node `Tx.defineEmbeddingModel`, `Tx.setEmbedding`, `View.vectorSearch` (`{hits, candidates, exact}`), `View.unembedded`; a model may be a plain IRI string; typed errors.

## 7. Tests and documentation (phase 1)

- [x] 7.1 Acceptance tests for every phase-1 scenario in `specs/vector-retrieval` and `specs/never-forget`: `crates/tiramemsu/tests/vector_recall.rs` (including budgets), `vector_properties.rs`, the bridge, MCP and binding suites, and the WASM host suites for "Every host".
- [x] 7.2 View fixture: now, as-of (with the embedding-time rule), history, valid-time, graph and predicate scopes in `vector_recall.rs`; brute-force rankings for now (with retractions, property test) and for now and as-of on every host (`tm-wasm/tests/common::check_vector_recall`).
- [x] 7.3 Property test (`vector_properties.rs`): exact recall equals brute force over random vectors, metrics, f32/f16 encodings, retractions and limits; prefilter hits carry exact similarities.
- [x] 7.4 Same results on the rusqlite host, on the WASM host's logic natively and in WebAssembly under Node.js (`tm-wasm` `native_host.rs` and `wasm_host.rs`, one brute-force-checked fixture).
- [x] 7.5 Format-5 migration test from earlier format fixtures; the schema check expects `embedding`, both indexes and both triggers.
- [x] 7.6 Benchmark: recall latency at 10⁴ and 10⁵ embeddings (384 f32, 768 f32, 768 f16), with and without prefilter; recorded in `crates/tiramemsu/benches/README.md`.
- [x] 7.6a Measured on-disk bytes per embedding (`examples/vector_footprint.rs`): 768 f32 4 234 B (table 4 105, `embedding_bits` 113, unique index 13), 768 f16 and 384 f32 about 2 150 B; recorded in `lat.md/storage.md#Measured Footprint`.
- [x] 7.6b Prefilter quality on real embeddings (GloVe-100 angular, 100k and 200k, 200 queries): recall@10 0.54 at n = 200, 0.79 at 1 000, 0.95 at 5 000; best speedup about 1.5×. Recorded in `crates/tiramemsu/benches/README.md` and `lat.md/query.md#Vector Recall`; 384/768-d sentence embeddings remain to be measured.
- [x] 7.7 `lat.md`: embeddings (storage), vector recall (query), format versioning, roadmap M7 status; run `lat check`.

## 8. Phase 2: query languages

- [x] 8.1 IR leaf `VectorPattern` and connection table function `tm_vector(model, vector, view, graphs, limit)` over `vector::search`; explain output.
- [x] 8.2 Cypher procedure `tiramemsu.vector.search(model, vector, {limit, graphs})` (model an IRI or a name resolved like a graph name) yielding `statement, subject, predicate, object, similarity, rank, confidence`, with time clauses.
- [x] 8.3 SPARQL group `tm:vectorMatch`, `tm:vectorModel` (both required), `tm:vectorSimilarity`, `tm:vectorRank`, `tm:vectorConfidence`, `tm:vectorLimit` with the `tm:text*` grouping rules (`GRAPH <g>` restricts, `SERVICE <tm:asOf/…>` times, `GRAPH ?g` rejected).
- [x] 8.4 Surface agreement: SPARQL and Cypher against `View::vector_search` (`vector_recall.rs`), bridge `vectorSearch` against bridge SPARQL and Cypher (`bindings/json/tests/bridge.rs`), and the MCP `vector_search` tool against an MCP SPARQL query (`tiramemsu-mcp/tests/server.rs`).
- [x] 8.5 Cypher `predicates` option and SPARQL `tm:vectorPredicate`, carried by `VectorPattern::predicates` into `tm_vector`; tested in `vector_properties.rs`.

## 9. Validation

- [x] 9.1 Validate with `openspec validate add-vector-recall --strict`; archive only after the open items are resolved or moved.
