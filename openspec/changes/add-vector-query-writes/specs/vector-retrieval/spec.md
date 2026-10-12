## ADDED Requirements

### Requirement: Dropping an embedding model
`Tx::drop_embedding_model(model)` SHALL retract every live definition statement of the model in one engine operation and return `true`, and SHALL return `false` and write nothing when the model is not defined. Stored embeddings SHALL NOT be deleted. After a drop, the now view SHALL treat the model as undefined (`set_embedding` and vector recall fail with `InvalidQuery`), while an as-of view before the drop SHALL still recall with it.

#### Scenario: Drop then search
- **WHEN** a model with embedded statements is dropped at transaction t2 and a recall runs now and as of t1 < t2
- **THEN** the now recall fails with `InvalidQuery` and the as-of recall returns the hits it returned before the drop

#### Scenario: Drop is idempotent
- **WHEN** `drop_embedding_model` runs for a model that is not defined
- **THEN** it returns `false` and the transaction writes nothing

#### Scenario: Identical redefinition restores
- **WHEN** a dropped model is defined again with its original definition
- **THEN** a now recall returns the embeddings stored before the drop

### Requirement: A model IRI keeps one definition
`define_embedding_model` SHALL fail with `InvalidQuery` when the model IRI ever had a different definition, live or dropped.

#### Scenario: Different redefinition after a drop
- **WHEN** a 4-dimension model is dropped and defined again with 8 dimensions
- **THEN** the call fails with `InvalidQuery`

### Requirement: Model definitions are read in the view
Vector recall SHALL read the model definition visible in their view: the live definition for now, the definition live at `t` for as of `t`, and the latest definition for history.

#### Scenario: Model defined after the view
- **WHEN** a model is defined at t2 and a recall runs as of t1 < t2
- **THEN** it fails with `InvalidQuery`

### Requirement: Vector writes from SPARQL Update
SPARQL Update SHALL define a model from `sys:dimensions` (required), `sys:metric` and `sys:vectorEncoding` templates on one subject (defaults cosine and f32), SHALL drop a model when a delete template matches one of its live definition statements, and SHALL store an embedding from a `tm:vectorEmbedding` string literal holding a JSON array together with one `tm:vectorModel` IRI on a subject that denotes a statement. An embedding subject that does not denote a statement, a second value for one definition or embedding predicate on a subject, or a `GRAPH` block around these templates SHALL fail with `InvalidQuery`; a `tm:vectorEmbedding` or `tm:vectorModel` template in a delete SHALL fail with `Unsupported`. The errors of the Rust operations SHALL apply unchanged.

#### Scenario: Define, assert and embed in one request
- **WHEN** one `INSERT DATA` defines `<urn:ex:m>` with 3 dimensions and inserts `v:a v:p v:b {| tm:vectorEmbedding "[1,0,0]" ; tm:vectorModel <urn:ex:m> |}`
- **THEN** the statement is live, its embedding is stored, and `tm:vectorMatch "[1,0,0]"` recalls it first

#### Scenario: Embed matched statements
- **WHEN** `INSERT { ?e tm:vectorEmbedding "[0,1,0]" ; tm:vectorModel <urn:ex:m> } WHERE { v:a v:p ?o ~ ?e }` runs
- **THEN** every matched statement gets that embedding

#### Scenario: Drop through DELETE DATA
- **WHEN** `DELETE DATA { <urn:ex:m> sys:dimensions 3 }` runs on a defined 3-dimension model
- **THEN** all three definition statements are retracted and the model is undefined

#### Scenario: Embeddings cannot be deleted
- **WHEN** a delete template has the predicate `tm:vectorEmbedding`
- **THEN** the request fails with `Unsupported` and nothing is written

### Requirement: Vector index commands and writes from Cypher
Cypher SHALL accept `CREATE VECTOR INDEX name [IF NOT EXISTS] OPTIONS map`, `DROP [VECTOR] INDEX name [IF EXISTS]` and `SHOW [VECTOR] INDEX[ES]` as whole queries, and the write procedure `tiramemsu.vector.setEmbedding(statement, model, vector) YIELD stored`. `CREATE` without `IF NOT EXISTS` on a defined model and `DROP` without `IF EXISTS` on an undefined model SHALL fail with `InvalidQuery`; `IF NOT EXISTS` with a different definition SHALL also fail with `InvalidQuery`. `OPTIONS` SHALL accept `dimensions`, `metric` and `encoding`, or Neo4j's `indexConfig` with `vector.dimensions` and `vector.similarity_function` (`euclidean` meaning `l2`). A `FOR … ON …` target SHALL fail with `Unsupported`. `CREATE`, `DROP` and `setEmbedding` SHALL be writes, refused on a read handle; `SHOW` SHALL list `name`, `model`, `dimensions`, `metric` and `encoding` of the models defined in the view.

#### Scenario: Create, embed, search, drop
- **WHEN** `CREATE VECTOR INDEX m OPTIONS {dimensions: 3}` runs, then `MATCH ()-[r:p]->() CALL tiramemsu.vector.setEmbedding(r, 'm', [1,0,0]) YIELD stored RETURN stored`, then a `tiramemsu.vector.search('m', [1,0,0])`, then `DROP INDEX m`
- **THEN** the first returns no rows, the second returns `true` per relationship, the search returns those relationships, and after the drop the search fails with `InvalidQuery`

#### Scenario: Read handle refuses DDL
- **WHEN** `CREATE VECTOR INDEX m OPTIONS {dimensions: 3}` is compiled for a read handle
- **THEN** it fails before anything runs

#### Scenario: Neo4j options
- **WHEN** `CREATE VECTOR INDEX m OPTIONS {indexConfig: {`vector.dimensions`: 8, `vector.similarity_function`: 'euclidean'}}` runs
- **THEN** the model has 8 dimensions, metric `l2` and encoding `f32`

#### Scenario: Languages agree
- **WHEN** one model is defined and embedded through SPARQL and another identically through Cypher, over the same statements
- **THEN** recall through either model returns the same statements in the same order
