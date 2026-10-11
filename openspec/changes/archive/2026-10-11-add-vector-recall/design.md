## Context

Status: implemented (unreleased, after 0.4.0; storage format 5), phases 1 and 2. The decisions below describe the implementation; Future work (V8) is not implemented.

Text recall (`lat.md/query.md#Text Recall`, `lat.md/storage.md#Text Index`) established the pattern this change follows: one logical operation in `tm-core` serving every surface; visibility through `scan_predicates` (the one writer of time predicates); predicate and graph-membership filters; evidence read in the same view; a versioned ranking policy with eid as the final tie-break; one budgeted operation.

The roadmap (`lat.md/roadmap.md#Milestones`, `docs/feature-roadmap.md` 0.6) specifies: definitions stored as `sys:` triples, embeddings supplied by the caller, hits that respect views and graphs exactly as text recall does. An earlier version of this change built on libSQL's DiskANN index through `add-libsql-host`; the decision of 2026-10-11 (user-approved) replaces it with the host-neutral design below. libSQL was decided against on 2026-10-11; `add-libsql-host` is deferred as an optional native host.

## Goals / Non-Goals

**Goals**

- Vector recall whose hits are exactly the statements the view selects, with the same evidence as text recall and no silent recall loss.
- Model definitions as auditable, versioned data; embeddings as caller data with a clear, append-only lifecycle.
- The same results on every SQLite host (rusqlite, WASM, any other) with no extension and no capability flag.
- One logical operation behind every surface, under the caller's budget.

**Non-Goals**

- Calling models, choosing models, or embedding on the caller's behalf.
- Versioned embeddings: an embedding describes one statement and never changes. (Its storage transaction is honoured by as-of views, V5.)
- Approximate indexes in this change (see Future work, with the rule each must follow).
- Fusion with text, graph expansion (`add-hybrid-recall`).

## Decisions

### V1. Exact first

Approximate nearest-neighbour indexes (HNSW, DiskANN, IVF) return a top-k first and filter it afterwards. Tiramemsu's views are filters: as-of and history select by transaction time, `valid_at` by valid time, graph scopes by membership, plus predicate and confidence filters. A top-k computed over every vector and then filtered by such a view can return fewer than `limit` hits, or the wrong ones, whenever the nearest vectors belong to statements the view does not select. Oversampling only makes the loss rarer, and there is no cheap way to tell the caller how much was lost.

Exact search inverts the order: SQL selects the visible rows, Rust scores every one of them. The hits are correct by construction under every view. At agent-memory scale it is fast enough: scoring is a dot product per row over values read sequentially from one table. Measured (Apple laptop, now view, top 10, pseudo-random cosine vectors): 11.6 ms for 10 000 × 768 f32 and 134.4 ms for 100 000 × 768 f32, about 1.2 µs per 768-dimension embedding (`crates/tiramemsu/benches/README.md`). So the first implementation is exact, and every later accelerator must keep exactness reachable (V8).

Size math (one vector per statement):

| Embedded statements × dimensions | f32 `vec` | f16 `vec` | `bits` |
|---|---|---|---|
| 100 000 × 768 | ≈ 300 MB | ≈ 150 MB | ≈ 10 MB (96 B each) |
| 100 000 × 384 | ≈ 150 MB | ≈ 75 MB | ≈ 5 MB |
| 10 000 × 1 536 | ≈ 60 MB | ≈ 30 MB | ≈ 2 MB |

The `bits` column is why the binary prefilter (V6) is cheap: a Hamming pass reads about 3 % of the bytes of an f32 scan, from the covering index `embedding_bits`.

### V2. Model definitions are `sys:` statements

A model is an IRI the caller names. The engine-recognised statements on it:

| Statement | Cardinality | Value |
|---|---|---|
| `(model sys:dimensions n)` | one | integer ≥ 1 |
| `(model sys:metric k)` | one | `"cosine"`, `"dot"` or `"l2"` |
| `(model sys:vectorEncoding e)` | one | `"f32"` or `"f16"` |

`sys:dimensions` (1 to 16 384), `sys:metric` and `sys:vectorEncoding` are engine-owned: only `define_embedding_model` writes them, and a user statement with one of them fails with `ReservedNamespace`. A model that is not an IRI is `InvalidTerm`. `Tx::define_embedding_model(model, &EmbeddingModel { dimensions, metric: Metric::{Cosine, Dot, L2}, encoding: VectorEncoding::{F32, F16} }) -> Result<bool>` writes all three: `true` when the model was not defined, `false` when an identical definition is already live (nothing written), and `InvalidQuery` when a different definition is live. A model whose definition must change is a new model IRI: vectors from different definitions are not comparable, and stored embeddings are immutable.

Alternatives considered: a configuration table (rejected: not auditable, does not time-travel or travel in bundles); a vector index node with `sys:embeds` predicates and value-keyed embeddings (the earlier design; replaced by statement-keyed embeddings, V3).

### V3. One embedding per (model, statement)

The key is the statement's eid. A statement is a fact, and agents embed the fact ("Alice works at Acme since 2024"), not an isolated value. Statement keys make the hit the statement itself, make visibility a plain join to `triple`, and let crypto-shredding find the rows of an erased subject's statements.

`Tx::set_embedding(eid, model, &[f32]) -> Result<bool>`:

- The statement must be live in the transaction (`NotLive` otherwise); the model must be defined (`InvalidQuery`); the length must equal `sys:dimensions` and every value must be finite (`InvalidQuery`).
- The vector is encoded per the model: unit-normalised when the metric is cosine (a zero vector is rejected as `InvalidQuery`), then stored as little-endian f32 or f16 in `vec`; `bits` holds one sign bit per dimension (`ceil(dim/8)` bytes, bit `i` set when value `i` > 0).
- If a row for (model, eid) exists with the same encoded `vec`, nothing is written and the call returns `false`. If it exists with a different `vec`, the call fails with `EmbeddingConflict`. Otherwise the row is inserted with `t` = the transaction and the call returns `true`.
- The insert belongs to the transaction: a failed transaction, a dry run or a speculation discards it.
- Supersede does not copy embeddings: the new statement has a new eid and a new content, so it needs its own vector. `View::unembedded(model, limit)` returns the visible statements without an embedding for that model (eid order), so callers can backfill.

### V4. Storage format 5

```sql
CREATE TABLE embedding (
  model INTEGER NOT NULL,  -- IRI ObjectId of the model
  eid   INTEGER NOT NULL,  -- STMT ObjectId
  t     INTEGER NOT NULL,  -- transaction that stored it
  bits  BLOB NOT NULL,     -- sign bits (ceil(dim/8) bytes), binary prefilter
  vec   BLOB NOT NULL      -- dim values, little-endian f32 or f16; unit-normalised when the metric is cosine
) STRICT;
CREATE UNIQUE INDEX embedding_model_eid ON embedding(model, eid);
-- covering index for the binary prefilter
CREATE INDEX embedding_bits ON embedding(model, eid, bits);
CREATE TRIGGER embedding_no_delete BEFORE DELETE ON embedding
BEGIN SELECT RAISE(ABORT, 'tiramemsu: embeddings are never deleted'); END;
CREATE TRIGGER embedding_no_update BEFORE UPDATE ON embedding
BEGIN SELECT RAISE(ABORT, 'tiramemsu: embeddings are immutable'); END;
```

- A 768-dimension f32 row is about 3.2 KB, roughly one 4 KiB page, so there are no `vec` overflow pages to skip: an exact scan reads about a page per row. The prefilter's win comes from the covering index `embedding_bits`, which holds `(model, eid, bits)` in about 100 bytes per row; phase one of V6 scans that index instead of the table.
- `embedding` is source data, not a derived index: it cannot be rebuilt from the graph, so it is protected like the graph (append-only). The only exception is crypto-shredding's erase transaction, which must delete the embedding rows of an erased subject's statements (an embedding is derived plaintext); that change specifies how the erase transaction lifts the guard.
- The 4→5 migration creates the table, both indexes and the triggers on every host and reads and writes no `triple`, `term` or `tx` row; a format-4 build refuses a format-5 file (`FormatVersion`).
- Plain SQLite types only: no `F32_BLOB`, no virtual table, no extension. `ANALYZE`, `integrity_check` and `VACUUM` work everywhere.

### V5. Recall: visible rows in SQL, similarity in Rust

```text
VectorQuery  { model, vector: Vec<f32>, limit (default 10), graphs, predicates,
               prefilter: Option<usize>, confidence: Option<predicate> }
VectorResult { hits: Vec<VectorHit>, candidates: u64, exact: bool }
VectorHit    { eid, s, p, o, similarity: f64, rank: u64, evidence }
```

1. Validate: the model must be given and defined in the view (`InvalidQuery` otherwise); the query length must equal its dimensions, every value must be finite and a cosine query must not be zero (`InvalidQuery`). The query is normalised for cosine and encoded like stored vectors for comparison.
2. SQL: `embedding e JOIN triple a ON a.eid = e.eid WHERE e.model = ?` plus the time predicates from `scan_predicates` (the only writer of time predicates), the `predicates` filter on `a.p`, and the graph-membership `EXISTS` of text recall, on the caller's snapshot. As-of, history, valid-time and graph views therefore select exactly the statements `View::triples` selects. The scan reads only `(eid, blob)`, with no `ORDER BY` and no per-row statement details or instant subquery.
   - **Embedding time.** An as-of view also requires `e.t <= t` (for an instant, the last transaction at or before it): it sees only the embeddings stored by its transaction, so a past view answers the same forever and saved answers on past views stay valid. Now and history views see every embedding. `View::unembedded` applies the same rule, so as of `t` it lists the statements that had no vector by then.
3. Rust scores each row, decoding into one reused buffer (f16 through a 65 536-entry lookup table): cosine = dot product of unit vectors; dot = dot product; l2 = negative Euclidean distance, so larger is always better. It keeps the best hits in a bounded heap.
4. Statement details and evidence are read only for the kept hits, with the same function as text recall: largest numeric confidence object (or the `confidence` predicate) or absent, `sys:confirmedBy` count, distinct `sys:author`s of the asserting and confirming transactions, `t_add` and `added_at`.
5. Ranking policy `tiramemsu-vector-rank/1`: similarity descending, confidence descending (absent last), confirmations descending, distinct authors descending, `added_at` newer first, eid ascending. Each hit gets a 1-based `rank`; `limit` cuts after ranking. Because evidence is read only for kept hits, ties in similarity at the cut are resolved by reading evidence for every row tied with the last kept similarity.
6. `candidates` = the number of visible embeddings scored; `exact = true`.
7. Budget: deadline and cancellation polled per row; returned hits charged as rows and bytes.

Alternative considered: computing similarity in SQL through a registered scalar function. Rejected: it requires the `functions` capability, which a minimal host may lack, and saves nothing (the bytes are read either way).

### V6. Optional binary prefilter

`prefilter: Some(n)` runs two phases. Phase one reads only `eid` and `bits` of the visible rows (from the covering index `embedding_bits`) and keeps the `n` rows with the smallest Hamming distance to the query's sign bits. Phase two reads `vec` for those `n` rows only and ranks them exactly (V5 steps 3 to 5). The result reports `exact: false` and `candidates` = the visible rows scanned in phase one. The view filter still runs first, so the prefilter never loses a hit to visibility, only to the binary approximation, and says so. Without a prefilter, `exact: true`. Measured with `n` = 200: 2.5 ms instead of 11.6 ms at 10 000 × 768 f32 and 31.3 ms instead of 134.4 ms at 100 000. Its recall of the true top k has not been measured on real embeddings; on the pseudo-random benchmark vectors it is not meaningful.

### V7. Entrypoints

- **Phase 1.** Rust (`Tx::define_embedding_model`, `Tx::set_embedding`, `View::vector_search`, `View::unembedded`); JSON bridge transaction ops `defineEmbeddingModel` and `setEmbedding` (the eid may be a named ref of the same transaction), read ops `vectorSearch` and `unembedded`; MCP tools `define_embedding_model` and `set_embedding` (write tools, hidden in read-only mode) and `vector_search` (read tool, bounded `limit`); Python `tx.define_embedding_model(model, dimensions, metric="cosine", encoding="f32")`, `tx.set_embedding(eid, model, vector)`, `view.vector_search(model, vector, limit=None, graphs=None, predicates=None, prefilter=None, confidence=None)` returning an iterable `VectorResult`, `view.unembedded(model, limit=None)`; Node `Tx.defineEmbeddingModel(model, dimensions, {metric, encoding})`, `Tx.setEmbedding(eid | Ref, model, vector)`, `View.vectorSearch(model, vector, {limit, graphs, predicates, prefilter, confidence})` returning `{hits, candidates, exact}`, `View.unembedded(model, limit?)`. In Python and Node a model may be a plain IRI string.
- **Phase 2.** IR leaf `VectorPattern` and connection table function `tm_vector(model, vector, view, graphs, limit)` over the same `vector::search`; Cypher `CALL tiramemsu.vector.search(model, vectorList, {limit, graphs}) YIELD statement, subject, predicate, object, similarity, rank, confidence` (model an IRI or a bare name resolved like a graph name) with the time clauses; SPARQL `?e tm:vectorMatch "[0.1, …]" ; tm:vectorModel <iri> ; tm:vectorSimilarity ?sim ; tm:vectorRank ?r ; tm:vectorConfidence ?c ; tm:vectorLimit 10` grouped on one subject variable, with the `tm:text*` grouping rules (`tm:vectorMatch` and `tm:vectorModel` required; `GRAPH <g>` restricts, `SERVICE <tm:asOf/…>` times it; `GRAPH ?g` around it is `InvalidQuery`).
- **No capability.** Every surface is available on every host.

### V8. Future work: accelerator tiers

Each tier is derived, never authoritative, and must follow these rules: it is built only from the `embedding` table; it can be dropped and rebuilt with no loss; a file written with it stays readable and writable by every host; it never changes which statements are visible; candidates it returns are re-ranked exactly on `vec`; and a result that may have missed hits reports `exact: false`.

- **sqlite-vec on native hosts** (not implemented). Optional derived tables in the same file (under the reserved `vec_` prefix), maintained in the write transaction where the host has the extension, with a `meta.vector_stale` catch-up marker exactly like `text_stale`: a host without the extension writes `embedding` only and marks the tier stale; the next capable writer catches up; until then search falls back to the exact path.
- **HNSW snapshots** (not implemented; usearch, or a pure-Rust implementation for WASM), built from the `embedding` table, optionally snapshotted together with a high-water-mark eid; rows above the mark are scanned exactly and merged. The snapshot is a cache outside the tiramemsu file, rebuilt or discarded at will. Its candidates are filtered through the view and re-ranked exactly; if fewer than `limit` visible hits survive, the search falls back to the exact path, so a time-travel query never returns fewer correct results than the exact path would.

Rejected alternatives:

- **libSQL DiskANN** (`add-libsql-host`, the earlier design). Ties vectors to one SQLite fork that lags upstream (3.45 to 3.47 versus 3.53), cannot run in WASM, needs non-STRICT `F32_BLOB` tables and an index expression that stock SQLite cannot evaluate (schema-wide `ANALYZE`, `integrity_check` and `VACUUM` then fail on stock hosts), and filters after top-k. Decided against on 2026-10-11 and deferred: libSQL may later host an approximate tier under V8's rules.
- **SQLite's `vec1`.** Pre-1.0 and without a WASM build at the time of the decision. Revisit as a V8 tier.
- **External vector stores** (Qdrant, LanceDB, pgvector). A second store breaks the one-file, one-transaction model: embeddings would not roll back with speculations, and view filtering would cross a process boundary.

## Risks / Trade-offs

- **Linear cost.** Exact search reads every visible embedding of the model: about 300 MB for 100 000 × 768 f32, measured at 134.4 ms (116.2 ms in f16, 31.3 ms with a 200-candidate prefilter). Mitigations: f16 encoding, the binary prefilter, view and predicate filters that shrink the scan, and the V8 tiers, the next step past roughly 10⁶ vectors.
- **Append-only vectors.** A wrong vector cannot be replaced for the same statement; the caller supersedes the statement (new eid) or uses a new model. Accepted: it keeps embeddings auditable and keeps the never-forget story simple.
- **Embeddings leak content.** A vector of a sealed or erased value reveals information. `add-crypto-shredding` must refuse embeddings of sealed statements and delete the embedding rows of an erased subject in the erase transaction.
- **f16 precision.** Similarities differ slightly from f32; results stay deterministic for a given file because both sides use the stored encoding.
- **Late embeddings are invisible to the past.** A vector stored after `t` is not seen as of `t`, even when the statement was live then. Accepted: it is what keeps past answers stable.
- **Large vectors through JSON** (MCP, bridge) are slow to parse. Accepted; Rust and the bindings take numeric arrays.

## Migration Plan

Format 4 → 5 (`v4_to_v5`) creates the empty `embedding` table, its unique and covering indexes and its two triggers; it runs on every host and reads and writes no graph row; a format-4 build refuses the result. New files are created as format 1 and migrated, as before. Default behavior and write cost are unchanged until a caller stores embeddings. Rollback is impossible by format rule (older builds refuse format 5).

## Open Questions

- Should embeddings travel in fact bundles (an optional section keyed by eid and model)?
- Should `View::unembedded` also skip statements whose predicate the caller never embeds (a per-model predicate allow-list as data), or is the `predicates` filter enough?
- Default prefilter size when callers enable it without a number (for example `10 × limit`)?
