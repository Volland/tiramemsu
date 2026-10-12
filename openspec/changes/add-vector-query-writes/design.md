## Context

`add-vector-recall` defines models as three engine-owned `sys:` statements and stores caller-supplied vectors append-only in `embedding`. SPARQL (`tm:vectorMatch`) and Cypher (`tiramemsu.vector.search`) can search; only Rust, the bridge, MCP and the bindings can write. This change adds the write half to both languages and the missing drop operation.

## Decisions

### W1. A vector index is a model

Neo4j users say "vector index", RDF users say "model". The store has one concept: a model IRI with dimensions, metric and encoding. Cypher DDL uses index words; SPARQL writes the definition statements themselves. Search is exact and needs no derived structure, so `CREATE VECTOR INDEX` writes a definition and nothing else.

### W2. Drop retracts the definition; vectors stay

`Tx::drop_embedding_model(model)` retracts every live `sys:dimensions`, `sys:metric` and `sys:vectorEncoding` statement of the model with the engine writer and returns whether a definition was live. The `embedding` rows are never deleted (never-forget). Consequences:

- Now view: the model is undefined; `set_embedding` and `vector_search` fail with `InvalidQuery` (`unembedded` keeps listing the backlog), as for a model that never existed.
- As-of views before the drop see the definition (W4) and the vectors stored by then, so past answers stay stable.
- Redefining the IRI with the identical definition restores it; the stored vectors become visible to now views again.

Alternative rejected: deleting the vectors. It would break as-of answers and the append-only guard, and gives no space back in an append-only file anyway.

### W3. One definition per IRI, for ever

`define_embedding_model` compares against the latest definition the IRI ever had, live or retracted. A different one is `InvalidQuery`. Stored vectors were encoded under that definition; reusing the IRI with other dimensions would make them undecodable. A new definition needs a new IRI.

### W4. Definitions are read in the view

`model_definition` takes the `ViewSpec`: now reads live statements; as of `t` reads the statements live at `t` (`t_add <= t AND (t_ret IS NULL OR t_ret > t)`, an instant resolved to its last transaction); history reads the latest definition ever made (fixed by W3).

### W5. SPARQL: intercept, don't invent syntax

SPARQL Update has no DDL, so the definition is written as the statements it is stored as:

```sparql
INSERT DATA { <urn:ex:minilm> sys:dimensions 384 ; sys:metric "cosine" ; sys:vectorEncoding "f32" }
DELETE DATA { <urn:ex:minilm> sys:dimensions 384 }          # drops the model
INSERT { ?e tm:vectorEmbedding "[0.1, …]" ; tm:vectorModel <urn:ex:minilm> }
WHERE  { v:alice v:worksAt v:acme ~ ?e }
INSERT DATA { v:alice v:worksAt v:acme {| tm:vectorEmbedding "[…]" ; tm:vectorModel <urn:ex:minilm> |} }
```

- Instantiated templates are partitioned per operation and solution: definition triples (`sys:dimensions`, `sys:metric`, `sys:vectorEncoding`), embedding triples (`tm:vectorEmbedding`, `tm:vectorModel`), and the rest.
- *Insert.* The rest is inserted first (so reifiers are bound), then each definition subject becomes one `define_embedding_model` (dimensions required, metric and encoding default to cosine and f32; at most one of each), then each embedding subject one `set_embedding` (exactly one `tm:vectorEmbedding`, a string literal holding a JSON array, and one `tm:vectorModel` IRI). The embedding subject must resolve to a statement: a `Value::Stmt`, or a blank reifier bound by the same insert.
- *Delete.* A definition triple whose content matches a live definition statement drops that model; a non-matching one is a no-op, like any absent triple. A `tm:vectorEmbedding` or `tm:vectorModel` template in a delete is `Unsupported`.
- *Graphs.* These templates name no graph membership; a `GRAPH` block around one is `InvalidQuery`.
- *Order.* Within one operation: delete, then rest-insert, definitions, embeddings. So `DELETE/INSERT` can drop and redefine identically, and one `INSERT DATA` can define a model, assert a statement and embed it.

### W6. Cypher: schema commands before the parser

`open-cypher` has no schema commands. `tm-cypher` recognises them on the token stream before parsing; a recognised command must be the whole query.

```text
CREATE VECTOR INDEX name [IF NOT EXISTS] OPTIONS map
DROP [VECTOR] INDEX name [IF EXISTS]
SHOW [VECTOR] INDEX[ES]
```

- `name` is an identifier or a backticked name, resolved like a graph name (`@vocab` or an IRI).
- `OPTIONS` is a literal map or a `$parameter`. Keys: `dimensions` (integer), `metric` (`cosine`, `dot`, `l2`), `encoding` (`f32`, `f16`), or Neo4j's `indexConfig` with `vector.dimensions` and `vector.similarity_function` (`cosine`, or `euclidean` meaning `l2`, scored as `-‖a−b‖`). Unknown keys are `Eval` errors.
- `FOR … ON …` is `Unsupported`.
- `CompileCtx::writable = false` refuses create and drop (they are writes); `SHOW` is a read.
- The compiled program carries `schema: Option<SchemaCommand>` and an empty query; `exec::run` executes it through `Runner::with_tx` or `run_ir`.

`CALL tiramemsu.vector.setEmbedding(statement, model, vector) YIELD stored` is a write clause (counted by `first_write`, refused on read handles): `statement` is a relationship or a statement node, `model` a name, `vector` a list of numbers; `stored` is `true` when the vector is new. A `null` argument skips the row.

## Risks

- Interception changes the meaning of a SPARQL delete of one definition statement (before: partial retraction; now: drop). The old behaviour was a corruption, so no user relied on it intentionally.
- Hand-parsing DDL duplicates a small part of the grammar; it is limited to three commands and fails with `Parse` spans for anything else in that shape.
