# Vector recall with evidence ranking (M7b)

## Status

Implemented (unreleased, after 0.4.0; storage format 5). Proposed 2026-10-10, redesigned 2026-10-11. Decision (user-approved, 2026-10-11): vector recall is **host-neutral**. Embeddings are a plain SQLite table in the tiramemsu file and search is exact, view-filtered and computed in Rust, so it runs on every SQLite host (rusqlite, WASM, any other). It does not depend on `add-libsql-host`: libSQL was decided against on 2026-10-11 and is deferred as an optional native host. Second half of milestone M7; the text half is `add-text-retrieval` (archived). Phase 1 (the Rust API, the JSON bridge, MCP and the Python and Node bindings) and phase 2 (the SPARQL and Cypher surfaces) are both implemented. Open items are tracked in `tasks.md` (WASM agreement test, brute-force property test, prefilter quality on real embeddings).

## Why

Agents recall by meaning more often than by words or graph patterns. Text recall finds "Lisbon offsite" only when the words match; an embedding of "the team trip to Portugal" should find it too. The store already holds everything needed to judge a hit (views, graphs, confidence layers, confirmations, authors, `tm:addedAt`); what it lacks is similarity search whose hits obey those same rules.

Approximate nearest-neighbour indexes rank first and filter afterwards, so under an as-of, valid-time or graph-scoped view they silently lose hits. Exact search over the visible rows cannot: it scores exactly the statements the view selects. At agent-memory scale (10⁴ to 10⁵ embedded statements) an exact scan is fast enough, needs no SQLite extension, and keeps every host equal. Faster tiers can be added later as derived, never-authoritative accelerators with exact re-ranking.

## What Changes

- **Model definitions as data.** An embedding model is an IRI described by `sys:` statements: `sys:dimensions` (integer), `sys:metric` (`"cosine"`, `"dot"` or `"l2"`), `sys:vectorEncoding` (`"f32"` or `"f16"`). They are versioned, time-travel and travel in bundles like every other statement. `Tx::define_embedding_model(model, &EmbeddingModel { dimensions, metric, encoding }) -> Result<bool>` writes them: `true` when newly defined, `false` for an identical redefinition, `InvalidQuery` for a different one (stored vectors depend on it). A non-IRI model is `InvalidTerm`; users cannot write these predicates directly (`ReservedNamespace`).
- **Caller-supplied embeddings, one per (model, statement).** `Tx::set_embedding(eid, model, &[f32]) -> Result<bool>` stores the vector of a live statement (`NotLive` otherwise; `InvalidQuery` for an undefined model, a wrong dimension, a non-finite component or a zero vector under cosine). The store never calls a model. An identical existing vector returns `false` (idempotent); a different vector for the same (model, eid) fails with `EmbeddingConflict`. Embeddings belong to their transaction (failed transactions, dry runs and speculations discard them). Supersede does not copy embeddings: the new statement needs its own; `View::unembedded(model, limit)` lists visible statements still lacking one, for backfill.
- **Storage format 5.** The 4→5 migration creates the STRICT table `embedding(model, eid, t, bits, vec)` with a unique index on `(model, eid)`, a covering index `embedding_bits` on `(model, eid, bits)` for the prefilter, and never-forget triggers that forbid `UPDATE` and `DELETE` on it. `vec` holds the values (little-endian f32 or f16, unit-normalised for cosine); `bits` holds the sign bits for an optional binary prefilter.
- **View-aware exact recall.** `View::vector_search(&VectorQuery) -> VectorResult` selects the visible (embedding ⨝ triple) rows through `scan_predicates`, the predicate filter and graph membership exactly as text recall does, computes similarity in Rust and keeps the top hits. Hits carry similarity, rank and the same evidence as text hits (read only for the hits). Ranking policy `tiramemsu-vector-rank/1`. An as-of view sees only the embeddings stored by its transaction (`embedding.t <= t`), so a past view answers the same forever; now and history see every embedding; `unembedded` follows the same rule. The result reports how many visible embeddings were scored and whether the search was exact. An optional binary prefilter (`prefilter: Some(n)`) keeps the `n` best candidates by Hamming distance on `bits`, then re-ranks them exactly; such a result reports `exact: false`.
- **Surfaces (phase 1).** JSON bridge transaction ops `defineEmbeddingModel`, `setEmbedding` (eid may be a ref) and read ops `vectorSearch`, `unembedded`; MCP tools `define_embedding_model`, `set_embedding` (write, hidden in read-only mode) and `vector_search`; Python `tx.define_embedding_model(model, dimensions, metric=, encoding=)`, `tx.set_embedding(eid, model, vector)`, `view.vector_search(model, vector, limit=, graphs=, predicates=, prefilter=, confidence=)` returning an iterable `VectorResult`, `view.unembedded(model, limit=None)`; Node `Tx.defineEmbeddingModel(model, dimensions, {metric, encoding})`, `Tx.setEmbedding(eid | Ref, model, vector)`, `View.vectorSearch(model, vector, {limit, graphs, predicates, prefilter, confidence})` returning `{hits, candidates, exact}`, `View.unembedded(model, limit?)`. In Python and Node a model may be a plain IRI string.
- **Surfaces (phase 2).** Cypher `CALL tiramemsu.vector.search(model, vectorList, {limit, graphs}) YIELD statement, subject, predicate, object, similarity, rank, confidence` (model an IRI or a bare name resolved like a graph name); SPARQL `?e tm:vectorMatch "[0.1, …]" ; tm:vectorModel <iri> ; tm:vectorSimilarity ?sim ; tm:vectorRank ?r ; tm:vectorConfidence ?c ; tm:vectorLimit 10` (`tm:vectorMatch` and `tm:vectorModel` required; `GRAPH <g>` restricts, `SERVICE <tm:asOf/…>` times it, `GRAPH ?g` around it is `InvalidQuery`). Both compile through the IR leaf `VectorPattern` to the table function `tm_vector(model, vector, view, graphs, limit)`.
- **No host capability.** Vector recall needs no `Capabilities` flag and works identically on every host.

## Capabilities

### New Capabilities

- `vector-retrieval`: embedding model definitions, caller-supplied embeddings, storage format 5, exact view-aware vector recall with evidence and optional binary prefilter, backfill listing, and its entrypoints.

### Modified Capabilities

- `never-forget`: the `embedding` table joins the guarded tables (no `UPDATE`, no `DELETE`), with the single exception reserved for crypto-shredding's erase transaction.

## Impact

`tm-core` (`vector` module, format-5 migration and DDL, `sys:dimensions`/`sys:metric`/`sys:vectorEncoding` vocabulary and validation, `EmbeddingConflict` error, shared recall evidence), facade (`Tx::define_embedding_model`, `Tx::set_embedding`, `View::vector_search`, `View::unembedded`, `EmbeddingModel`, `Metric`, `VectorEncoding`, `VectorQuery`, `VectorHit`, `VectorResult`), JSON bridge, MCP server, Python and Node bindings, tests and `lat.md`; and for phase 2 `tm-ir` (`VectorPattern`), `tm-exec` (`tm_vector`), `tm-sparql`, `tm-cypher`. Preserve statement identity, never-forget invariants, per-view visibility and current default behavior: without embeddings nothing changes except the format number.

## Non-goals

- No embedding model, provider call or tokenizer in the store, and no automatic embedding of new statements.
- No approximate index in this change: no sqlite-vec, no HNSW, no libSQL DiskANN (see design, Future work).
- No hybrid fusion or graph expansion (that is `add-hybrid-recall`).
- No embeddings in fact bundles in this change.
- No versioning of vectors: an embedding is immutable once stored and describes one statement. Its storage transaction is honoured by as-of views, but a vector is never replaced.
