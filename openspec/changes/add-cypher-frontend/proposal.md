## Why

Tiramemsu promises that SPARQL and Cypher query one store with the same results (`lat.md/overview#Goals`, decision D13). Milestone M1 (`add-query-ir-and-sql-planner`) provides the shared IR and SQL planner, but there is no Cypher front end yet. Agent-memory users think in labelled property graphs, and they need Cypher to reach the engine's distinctive features: addressable statements (layers), bitemporal time travel and per-pattern time scopes. This change is milestone M2b in `lat.md/roadmap#Milestones`, and it can run in parallel with M2a (`add-sparql-frontend`).

## What Changes

- New crate `tm-cypher`. It parses an openCypher subset plus the documented Tiramemsu extensions, runs semantic analysis, lowers to the `tm-ir` algebra with `graph_set = BagOfEids`, `match_mode = RelIsomorphism` and `missing = Null3VL`, and encodes results as Cypher values.
- Read clauses: `MATCH`, `OPTIONAL MATCH`, `WHERE`, `WITH`, `RETURN`, `ORDER BY`/`SKIP`/`LIMIT`, `UNWIND`, aggregates, `UNION`, `EXISTS {}`, and `CALL { … }` subqueries (uncorrelated, or importing `WITH`). The rest of openCypher parses but reports `Unsupported`. Relationship isomorphism is the default, and `MATCH REPEATABLE ELEMENTS` opts out.
- Write clauses mapped onto the M0 transaction engine: `CREATE` → create (always a new eid, so parallel edges are possible), `MERGE` → unique-key upsert or an atomic pattern match in the writer transaction, `SET` → assert or supersede, `REMOVE`/`DELETE` → retract with cascade, and `DETACH DELETE` → retract every statement that mentions the node. `DELETE` of a node that still has relationships fails, as in Neo4j.
- Cypher dual view (Q14): a statement eid is both a relationship and a node with the implicit label `:Statement`. Relationship variables may stand in node position, which is how Cypher reads and writes layers.
- Temporal syntax (D14): `USE AS OF <tx>|datetime(…)`, `USE VALID AT …`, `USE HISTORY`, per-pattern scopes through `CALL { USE … }`, and the statement properties `txAdded`, `txRetracted`, `validFrom` and `validTo`.
- Vocabulary mapping (Q15): Cypher names map to and from IRIs through `@vocab` and the versioned prefix table, with backticked CURIEs and full IRIs. Labels are `rdf:type`, and `sys:` is hidden from `labels()`, `keys()` and `properties()`.
- Volatile values show as virtual node properties under the `Now` view. A stored triple wins on a name collision.
- Facade entry points: `View::cypher(q, params)` for read-only queries, and `Tx::cypher(q, params)` plus the convenience `Db::cypher_write(opts, q, params)` for queries that write.
- A cross-dialect differential test suite: equivalent SPARQL/Cypher query pairs over shared fixtures must return identical normalised results, and the dual view and SPARQL `~ ?r` must bind the same eid. This suite needs `add-sparql-frontend` to be complete as well.
- Variable-length relationships, `shortestPath` and `allShortestPaths` are parsed here but return `Unsupported` until `add-path-engine` (M3) modifies this capability to evaluate them.
- Two new typed errors, `DeleteConnectedNode` and `Eval`, are added to the facade error set. Nothing existing breaks (greenfield).

## Capabilities

### New Capabilities
- `cypher-read`: Read clauses and their semantics (patterns, labels, statement classification, property access, isomorphism, 3VL NULLs, projection, ordering, aggregation, subqueries, parameters), the result value model, volatile virtual properties, unsupported features, and parse, compile and evaluation errors.
- `cypher-write`: Write clauses (`CREATE`, `MERGE`, `SET`, `REMOVE`, `DELETE`, `DETACH DELETE`), their mapping to transaction operations, the write entry points, and transactional failure behaviour.
- `cypher-dual-view`: Statements as both relationships and `:Statement` nodes, relationship variables in node position, layer reads and writes, and `startNode`/`endNode`/`type` on either form.
- `cypher-temporal-clauses`: `USE AS OF`, `USE VALID AT` and `USE HISTORY` at query level, per-pattern `CALL { USE … }` scopes, and statement time properties (`txAdded`, `txRetracted`, `validFrom`, `validTo`).
- `vocabulary-mapping`: Cypher names ↔ IRIs through `@vocab` and the prefix table, labels ↔ `rdf:type`, node IRIs (`@id`, `elementId`), reserved implicit labels, and hiding of the `sys:` namespace.
- `dialect-differential-testing`: The SPARQL/Cypher equivalence corpus, fixtures, result normalisation, and the dual-view eid equivalence check.

### Modified Capabilities
- None. No capability specs are archived yet (`openspec/specs/` is empty). The M1 capabilities this change builds on are consumed, not changed.

## Impact

- **New crate:** `crates/tm-cypher` (depends on `tm-ir`, `tm-core` ids and values, and the chosen parser crate `open-cypher`, pinned; see design.md).
- **Facade (`tiramemsu`):** adds `View::cypher`, `Tx::cypher` and `Db::cypher_write`, the Cypher variant of `QueryResult` with its JSON encoding, the `Params` type, the error variants `DeleteConnectedNode` and `Eval`, and a Cypher program executor that interleaves `tm-exec` read plans with `Tx` operations.
- **IR (`tm-ir`, owned by M1):** consumed as is. The lowering relies on correlated scalar lookups, `Exists`/`NotExists`, and null-safe joins. Any gap is added in coordination with `add-query-ir-and-sql-planner`, not forked. See design.md, Risks.
- **Tests:** an openCypher TCK subset runner, unit and integration tests per capability, and the differential corpus in the facade crate. The differential suite requires `add-sparql-frontend`.
- **Docs:** `lat.md/query.md`, `lat.md/data-model.md` and `lat.md/api.md` get `@lat` code references and record the decisions made while writing this spec.
- **Dependencies:** requires M0 (`add-core-store`) and M1 (`add-query-ir-and-sql-planner`). Runs in parallel with M2a (`add-sparql-frontend`). M3 (`add-path-engine`) will later modify `cypher-read` to evaluate path patterns.
