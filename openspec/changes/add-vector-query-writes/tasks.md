## 1. Core

- [x] 1.1 `Tx::drop_embedding_model` in `crates/tm-core/src/engine/embedding.rs`.
- [x] 1.2 `define_embedding_model` compares against the latest definition ever stored for the IRI.
- [x] 1.3 `vector::model_definition` takes the `ViewSpec`; `search` requires the model in its view.

## 2. SPARQL

- [x] 2.1 `TM_VECTOR_EMBEDDING` in `tm-ir` vocabulary.
- [x] 2.2 Interception of definition and embedding templates in `crates/tm-sparql/src/update/run.rs` (insert, delete, `GRAPH` check).

## 3. Cypher

- [x] 3.1 Schema-command recogniser (`crates/tm-cypher/src/parse/schema.rs`) and `CypherProgram::schema`.
- [x] 3.2 Execution of create, drop and show in `crates/tm-cypher/src/exec/schema.rs`.
- [x] 3.3 `tiramemsu.vector.setEmbedding` write procedure.

## 4. Tests

- [x] 4.1 Core: drop, idempotent drop, identical redefinition restores, different redefinition refused, as-of definition.
- [x] 4.2 SPARQL: define/assert/embed in one request, embed matched statements, drop by `DELETE DATA`, delete of an embedding refused.
- [x] 4.3 Cypher: create/embed/search/drop, read handle refuses DDL, Neo4j options, `IF [NOT] EXISTS`, `SHOW`.
- [x] 4.4 SPARQL and Cypher writes agree.

## 5. Documentation

- [x] 5.1 `lat.md/query.md#Vector Recall`, tests in `lat.md/tests.md`; `lat check`; `openspec validate`.
- [ ] 5.2 Follow-up: `dropEmbeddingModel` on the bridge, MCP and bindings.
