# vector-retrieval Specification

## Purpose
Vector recall over caller-supplied embeddings: models defined as `sys:` statements, vectors stored append-only in the `embedding` table of storage format 5 inside the write transaction, and an exact, view-aware search (as-of views see only the embeddings stored by then) with an optional sign-bit prefilter. One search serves Rust, SPARQL, Cypher, the JSON bridge, MCP and the bindings, on every SQLite host. Archived from `openspec/changes/archive/2026-10-11-add-vector-recall`.

## Requirements

### Requirement: Embedding model definitions are statements
An embedding model SHALL be an IRI defined by exactly one live `sys:dimensions` (integer from 1 to 16 384), `sys:metric` (`"cosine"`, `"dot"` or `"l2"`) and `sys:vectorEncoding` (`"f32"` or `"f16"`) statement. `Tx::define_embedding_model(model, &EmbeddingModel { dimensions, metric, encoding })` SHALL write these statements and return `true` when the model was not defined, SHALL write nothing and return `false` when an identical definition is live, and SHALL fail with `InvalidQuery` when a different definition is live. A model that is not an IRI SHALL fail with `InvalidTerm`. The three predicates SHALL be engine-owned: a user statement with one of them SHALL fail with `ReservedNamespace`. Definitions SHALL be versioned like every other statement.

#### Scenario: New definition
- **WHEN** `define_embedding_model` is called for an undefined model with 4 dimensions, cosine and f32
- **THEN** it returns `true` and `View::triples` shows the three `sys:` statements on the model IRI

#### Scenario: Identical redefinition
- **WHEN** the same definition is written again
- **THEN** it returns `false` and no statement is added

#### Scenario: Different redefinition
- **WHEN** a defined 4-dimension model is defined again with 8 dimensions
- **THEN** the call fails with `InvalidQuery` and the transaction leaves no trace

#### Scenario: Reserved definition predicates
- **WHEN** a user transaction asserts `sys:dimensions` on an IRI directly
- **THEN** it fails with `ReservedNamespace`

### Requirement: Caller-supplied statement embeddings
The store SHALL NOT compute embeddings. `Tx::set_embedding(eid, model, vector)` SHALL store at most one vector per model and statement. The statement SHALL be live (`NotLive` otherwise), the model SHALL be defined, the vector length SHALL equal the model's dimensions, every value SHALL be finite and a vector under a cosine model SHALL NOT be zero (`InvalidQuery` otherwise). The vector SHALL be stored in the model's encoding, unit-normalised when the metric is cosine, together with its sign bits. Setting a vector identical to the stored one SHALL return `false` and write nothing; setting a different vector for the same model and statement SHALL fail with `EmbeddingConflict`; a new embedding SHALL return `true`. Embeddings SHALL belong to their transaction, so failed transactions, dry runs and speculations discard them.

#### Scenario: Idempotent write
- **WHEN** the same vector is set twice for one statement and model
- **THEN** the first call returns `true`, the second returns `false`, and one row exists

#### Scenario: Conflicting write
- **WHEN** a different vector is set for a statement and model that already have one
- **THEN** the call fails with `EmbeddingConflict` and the stored vector is unchanged

#### Scenario: Wrong dimension
- **WHEN** a 3-value vector is set for a model of 4 dimensions
- **THEN** the call fails with `InvalidQuery` and the transaction leaves no trace

#### Scenario: Retracted statement
- **WHEN** an embedding is set for a statement that has been retracted
- **THEN** the call fails with `NotLive`

#### Scenario: Speculation discards embeddings
- **WHEN** an embedding is set inside a speculative block or a dry run
- **THEN** afterwards it is absent from the store and from vector recall

### Requirement: Backfill after supersede
Supersede SHALL NOT copy embeddings to the new statement. `View::unembedded(model, limit)` SHALL list the statements visible in the view that have no embedding for the model.

#### Scenario: Superseded statement needs a new embedding
- **WHEN** an embedded statement is superseded
- **THEN** the new statement has no embedding and `View::unembedded` for that model lists it, while the old statement's embedding is unchanged

### Requirement: Storage format 5
Storage format 5 SHALL add the STRICT table `embedding(model, eid, t, bits, vec)`, with `bits` before `vec`, the unique index `embedding_model_eid` on `(model, eid)`, the covering index `embedding_bits` on `(model, eid, bits)` for the prefilter, and the triggers `embedding_no_delete` and `embedding_no_update`. The 4→5 migration SHALL run on every host and SHALL read and write no `triple`, `term` or `tx` row. A format-4 build SHALL refuse a format-5 file.

#### Scenario: Migration preserves the graph
- **WHEN** a format-4 file is opened by a build that supports format 5
- **THEN** it is migrated to format 5 with every term, statement and transaction row unchanged, the `embedding` table is empty, and a format-4 build then refuses the file with `FormatVersion`

#### Scenario: STRICT embeddings
- **WHEN** a raw SQLite connection inserts a TEXT value into the `vec` column of `embedding`
- **THEN** SQLite rejects the insert with a datatype error

### Requirement: Exact view-aware vector recall
`View::vector_search(&VectorQuery)` SHALL score every embedding of the model whose statement is visible in the view, selected through the same time predicates, predicate filter and graph-membership test as text recall, in one snapshot. Similarity SHALL be the dot product of unit vectors for cosine, the dot product for dot, and the negative Euclidean distance for l2, so larger is always better. The result SHALL report the number of visible embeddings scored and `exact = true`. A missing or unknown model, a vector of the wrong length, a non-finite value or a zero vector under cosine SHALL fail with `InvalidQuery`. Vector recall SHALL require no host capability.

#### Scenario: Retracted statement
- **WHEN** a statement whose embedding is nearest to the query is retracted and a now recall runs
- **THEN** it is absent from now recall but present in an as-of view before the retraction

#### Scenario: Graph and valid time
- **WHEN** a recall selects a graph and a valid instant
- **THEN** every hit is a member of that graph in the view and valid at that instant, and the hits equal the top of a brute-force ranking over exactly the statements `View::triples` returns for that view

#### Scenario: Limit is filled under a narrow view
- **WHEN** the nearest embeddings belong to statements outside the view and at least `limit` visible statements are embedded
- **THEN** the recall still returns `limit` hits, all visible

#### Scenario: Every host
- **WHEN** the same recall runs on the rusqlite host and in WebAssembly over the same file
- **THEN** both return the same hits in the same order

### Requirement: Embeddings are visible from the transaction that stored them
An as-of view SHALL see only the embeddings whose storing transaction is at or before the view's transaction (`embedding.t <= t`; for an instant, the last transaction at or before it), so a past view returns the same hits forever and a saved answer on a past view stays valid. Now and history views SHALL see every stored embedding. `View::unembedded` SHALL apply the same rule: in an as-of view it SHALL list a visible statement that had no embedding for the model by the view's transaction.

#### Scenario: Embedding stored after the view's transaction
- **WHEN** a statement is asserted at transaction t1 and its embedding is stored at a later transaction t2
- **THEN** a vector recall as of t1 does not return it and `View::unembedded` as of t1 lists it, while a now recall returns it and `View::unembedded` in the now view does not list it

#### Scenario: Past answers are stable
- **WHEN** the same as-of recall runs before and after new embeddings are stored for statements visible in that view
- **THEN** both runs return the same hits, similarities and ranks

### Requirement: Auditable vector ranking
Each hit SHALL carry its statement id, subject, predicate, object, similarity, 1-based rank and the same evidence components as text recall (confidence or its absence, confirmations, distinct authors, `t_add`, `added_at`), read in the query's view. Hits SHALL be ordered by the policy `tiramemsu-vector-rank/1`: similarity descending, confidence descending with absent last, confirmations descending, distinct authors descending, `added_at` newer first, statement id ascending.

#### Scenario: Evidence breaks a similarity tie
- **WHEN** two hits have equal similarity and one has confidence 0.9 while the other has none
- **THEN** the hit with confidence 0.9 is ranked first

#### Scenario: Missing confidence
- **WHEN** a hit has no confidence layer
- **THEN** its confidence is reported as absent rather than estimated

#### Scenario: Deterministic
- **WHEN** the same recall runs twice on an unchanged database
- **THEN** both runs return the same hits, similarities and ranks

### Requirement: Optional binary prefilter
When a query sets `prefilter` to `n`, recall SHALL first keep the `n` visible embeddings with the smallest Hamming distance between their sign bits and the query's, then rank only those exactly, and SHALL report `exact = false`. The view filter SHALL apply before the prefilter.

#### Scenario: Prefiltered result is marked inexact
- **WHEN** a recall runs with a prefilter
- **THEN** every hit is visible in the view, its similarity equals the exact similarity for that statement, and the result reports `exact = false`

### Requirement: One vector recall from every surface
The Rust API, the JSON bridge (`defineEmbeddingModel`, `setEmbedding`, `vectorSearch`, `unembedded`), the MCP tools (`define_embedding_model`, `set_embedding`, `vector_search`) and the Python and Node bindings SHALL run the same logical operations, so equal requests return the same hits in the same order, and each recall SHALL be one operation under the caller's query budget. The MCP write tools SHALL be hidden in read-only mode. Cypher (`CALL tiramemsu.vector.search(model, vector, {limit, graphs})`, the model an IRI or a name resolved like a graph name) and SPARQL (a `tm:vectorMatch` group with the required `tm:vectorModel` and the optional `tm:vectorSimilarity`, `tm:vectorRank`, `tm:vectorConfidence` and `tm:vectorLimit`) SHALL run the same recall through the IR leaf `VectorPattern` and the table function `tm_vector`. In SPARQL, `GRAPH <g>` SHALL restrict the recall, `SERVICE <tm:asOf/…>` SHALL time it, and `GRAPH ?g` around it SHALL fail with `InvalidQuery`.

#### Scenario: Surfaces agree
- **WHEN** the same vector is recalled through `View::vector_search`, the bridge `vectorSearch`, the MCP `vector_search` tool and the Python and Node bindings
- **THEN** all return the same statements, similarities and ranks

#### Scenario: Query languages agree
- **WHEN** the same vector and model are recalled through a SPARQL `tm:vectorMatch` group, Cypher `tiramemsu.vector.search` and `View::vector_search`
- **THEN** all return the same statements in the same order

#### Scenario: Variable graph around a vector group
- **WHEN** a SPARQL query wraps a `tm:vectorMatch` group in `GRAPH ?g`
- **THEN** it fails with `InvalidQuery`

#### Scenario: Bridge reference
- **WHEN** a bridge transaction asserts a statement under a named ref and sets its embedding by that ref in the same op list
- **THEN** the embedding is stored for the new statement

#### Scenario: Read-only MCP
- **WHEN** the MCP server runs in read-only mode
- **THEN** `vector_search` is listed and `define_embedding_model` and `set_embedding` are not

#### Scenario: Budgeted recall
- **WHEN** a recall runs under a budget whose row limit is smaller than its hits, or whose token is cancelled
- **THEN** it fails with `ResultLimitExceeded` or `Cancelled` and returns no partial result
