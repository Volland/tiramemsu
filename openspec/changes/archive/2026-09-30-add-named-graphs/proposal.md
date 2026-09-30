## Why

Tiramemsu v1 has no named graphs: SPARQL `GRAPH`, `FROM <g>` and `FROM NAMED <g>` fail with `Unsupported("named graph")` (`sparql-temporal-dataset`, `sparql-update`, `sparql-query`). Agent memory needs a way to group statements (a session, a source document, an agent's beliefs, an import batch) and to say things about the group: who created it, when, how far to trust it, what it is for. RDF datasets and every SPARQL client expect that grouping to be spelled `GRAPH <g>`.

Decision D3 rules out a named-graph key column, and the design already reserves the syntax: `SERVICE` scopes time so that `GRAPH` stays free (D21). Because every statement has an eid, a graph does not need a column. Membership can be a layer triple on the eid, and the graph is an ordinary node, so metadata on a graph is ordinary triples. This change specifies that model and the SPARQL surface for it. It is a specification only; nothing is implemented here.

## What Changes

- A **named graph** is a node (an `IRI`, `NODE` or `BNODE` id) that is the object of one or more live membership statements `(eid, sys:inGraph, g)`. Membership is itself a statement with its own eid, its own lifetime and its own valid time, so it is bitemporal and can carry layers (`who added it, and why`).
- **Graph metadata** is ordinary triples with the graph node as subject: `(v:session12 v:startedBy v:agent7)`. Nothing is new in storage or in the ObjectId encoding, and the storage format version does not change.
- A statement can be a member of any number of graphs, or of none. **Graphs are tags on statements, not containers**: one statement has one eid however many graphs it is in.
- The **default graph is the union** of all visible statements, as it is today. `GRAPH <g>` and `GRAPH ?g` select by membership. `FROM <g>` and `FROM NAMED <g>` work as dataset clauses and combine with the `tm:` time IRIs.
- **SPARQL Update** gains `GRAPH` blocks in `INSERT DATA`, `DELETE DATA` and templates, plus `WITH` and `USING`. Adding to a graph asserts the statement idempotently and asserts a membership. Deleting from a graph retracts the membership only. `CREATE GRAPH`, `CLEAR GRAPH`, `CLEAR NAMED`, `DROP GRAPH` and `DROP NAMED` are supported. `LOAD`, `ADD`, `MOVE`, `COPY`, `CLEAR DEFAULT` and `CLEAR ALL` stay unsupported.
- **Rust API:** `Tx::add_to_graph`, `Tx::remove_from_graph`, `Tx::clear_graph`, `View::graphs` and `View::graph_members`.
- New typed error `InvalidGraphName`. The reserved-predicate rule is extended so that only the engine writes `sys:inGraph`.
- Removed behaviour: the three `Unsupported("named graph")` and `Unsupported("GRAPH variable")` rejections, and the update-side rejections of `GRAPH`, `WITH`, `USING` with a graph IRI.

## Capabilities

### New Capabilities
- `named-graphs`: The graph and membership model, graph metadata, dataset semantics (default graph, `FROM`, `FROM NAMED`, `GRAPH`), membership in updates, graph management operations, time and valid-time behaviour, the Rust API, errors, and the interaction with layers and the cascade.

### Modified Capabilities
- `sparql-temporal-dataset`: removes "Non-time graphs are unsupported". `FROM`, `FROM NAMED` and `GRAPH` with a non-`tm:` IRI now name graphs.
- `sparql-update`: `GRAPH` blocks, `WITH`, `USING` and the graph-management operations are supported as specified in `named-graphs`. `LOAD`, `ADD`, `MOVE`, `COPY`, `CLEAR DEFAULT` and `CLEAR ALL` stay unsupported.
- `sparql-query`: `GRAPH ?g` and graph IRIs are no longer in the "unsupported" list.

## Impact

- **`tm-core`:** the `sys:inGraph` predicate and the `sys:Graph` class, the membership operations on `Tx`, the `InvalidGraphName` error, and a graph-membership check in the reserved-namespace rule. No table or index change is expected (the live `(p, o)` index serves `GRAPH <g>`); design.md, Risks, states how that is verified.
- **`tm-ir`:** `TriplePattern` gains a graph selector (`Any`, `Set(ids)` or `Var`), so `GRAPH` lowers to an extra membership join without a new operator. The IR is owned by M1, so the change is made in coordination with it and not forked.
- **`tm-sparql`:** the parser already accepts `GRAPH`; lowering, dataset handling, update templates and graph management change.
- **Facade (`tiramemsu`):** the methods above, the error variant, and the MCP `graph` argument on write and search tools.
- **Not affected:** `tm-cypher` (`USE GRAPH` is out of scope), the ObjectId encoding, the storage format version, time semantics of existing views.
- **`add-path-engine`:** property paths inside `GRAPH` or under `FROM <g>` are a documented limit here (see design.md, Open Questions).
- **Docs:** `lat.md/data-model.md` (Layers, the `sys:inContext` example), `lat.md/query.md` (Temporal Syntax, "v1 has no named graphs") and `lat.md/overview.md` (a new decision row) change when this is implemented, not now, because `lat.md/` holds decided and built design only.
- **Dependencies:** requires M0, M1 and M2a (`add-sparql-frontend`, archived). It is independent of `add-cypher-frontend`.
