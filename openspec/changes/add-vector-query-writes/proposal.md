# Vector writes and index DDL from SPARQL and Cypher

## Status

Proposed and implemented 2026-10-12. Follows `add-vector-recall` (archived 2026-10-11), which gave SPARQL and Cypher vector *search* but left model definitions and embeddings writable only from Rust, the JSON bridge, MCP and the bindings.

## Why

A query-language user can recall by meaning but cannot store the vectors to recall, nor define or remove the model those vectors belong to, without leaving the language. Agents and tools that speak only SPARQL or only Cypher (a SPARQL endpoint client, a Cypher console, an LLM generating queries) need the whole loop: define a vector index, embed statements, search, and drop the index when it is retired.

The store also has a gap: the three `sys:` definition statements of a model are engine-owned for writes, but not for retraction. A SPARQL `DELETE DATA { <m> sys:dimensions 4 }` today retracts one of them and leaves a half-defined model. Dropping a model needs one operation that removes the whole definition.

## What Changes

- **Vector index = embedding model.** In both languages a *vector index* is an embedding model IRI with its definition (`sys:dimensions`, `sys:metric`, `sys:vectorEncoding`). Search stays exact; creating an index builds no derived structure, it defines the model that embeddings and searches name.
- **Drop.** `Tx::drop_embedding_model(model) -> Result<bool>` retracts the live definition as one engine operation (`true` when it was defined). Embeddings stay stored (append-only); in the now view the model is undefined, so `set_embedding` and search fail with `InvalidQuery`. Views before the drop still see the model and its vectors.
- **Definitions are fixed for the life of an IRI.** Redefining a model, live or dropped, with a different definition is `InvalidQuery`; an identical redefinition after a drop restores it, with its stored vectors.
- **Time-travel-correct definitions.** Search reads the model definition in the query's view (as of `t`: the definition live at `t`), no longer always the live one.
- **SPARQL Update.**
  - *Create:* `INSERT DATA { <m> sys:dimensions 384 ; sys:metric "cosine" ; sys:vectorEncoding "f32" }` (metric and encoding optional, defaulting to cosine and f32) defines the model; the three statements are written by the engine as one definition.
  - *Drop:* deleting any live definition statement of a model (`DELETE DATA`, or a `DELETE … WHERE` template) drops the whole model.
  - *Insert embedding:* `tm:vectorEmbedding "[…]"` with `tm:vectorModel <m>` on one subject that denotes a statement: a bound `?e`, a statement IRI, a reifier (`<< s p o >>`, `~ _:r`, annotation `{| … |}`) whose statement the same request asserts or names.
  - Deleting a `tm:vectorEmbedding` triple is `Unsupported` (embeddings are append-only); a `GRAPH` block around any of these templates is `InvalidQuery`.
- **Cypher.**
  - `CREATE VECTOR INDEX name [IF NOT EXISTS] OPTIONS {dimensions: 384, metric: 'cosine', encoding: 'f32'}` (also Neo4j's `indexConfig: {`vector.dimensions`, `vector.similarity_function`}`); without `IF NOT EXISTS` an existing model is `InvalidQuery`.
  - `DROP [VECTOR] INDEX name [IF EXISTS]`; without `IF EXISTS` an undefined model is `InvalidQuery`.
  - `SHOW [VECTOR] INDEXES` returns `name, model, dimensions, metric, encoding` for the models defined in the view.
  - `CALL tiramemsu.vector.setEmbedding(statement, model, vector) YIELD stored` stores an embedding for a relationship (or statement node); it is a write clause.
  - Names resolve like the `model` argument of `tiramemsu.vector.search` (a bare name through `@vocab`, or an IRI).

## Capabilities

### Modified Capabilities

- `vector-retrieval`: model drop, definitions fixed per IRI, view-correct definitions, and the SPARQL Update and Cypher write surfaces (the update interception and the Cypher DDL are specified there, beside the search they serve).

## Impact

`tm-core` (`Tx::drop_embedding_model`, history check in `define_embedding_model`, view-aware `model_definition`), facade re-exports, `tm-ir` vocabulary (`tm:vectorEmbedding`), `tm-sparql` update runner, `tm-cypher` (schema-command parser, `CypherProgram::schema`, procedure), tests and `lat.md`. No storage format change. Existing behaviour is unchanged except that a partial SPARQL delete of a model definition now drops the whole model, and a dropped-then-differently-redefined IRI is refused.

## Non-goals

- No approximate index: `CREATE VECTOR INDEX` does not build HNSW or sqlite-vec tables (see `add-vector-recall` design V8).
- No `FOR (n:Label) ON (n.prop)` index targets: embeddings belong to statements, not node properties; such a clause is `Unsupported`.
- No deletion or replacement of stored vectors.
- No `dropEmbeddingModel` on the JSON bridge, MCP or the bindings in this change (follow-up).
