## Context

See `proposal.md` (Why) for the motivation. This section covers the state and constraints that shape the design.

- **Statements have identity.** Every fact is a row `(eid, s, p, o)` with its own lifetime and valid time, and eids can be the subject or object of other triples (`lat.md/data-model#Layers`). Layers are how provenance, confidence and context are attached.
- **D3:** "no named-graph key column". `lat.md/data-model#Layers` says that context, session or agent membership is a layer triple on the eid, e.g. `(e1 sys:inContext :session12)`, and that physical isolation per agent means one SQLite file per agent.
- **D21:** SPARQL scopes time with `SERVICE <tm:…>` so that `GRAPH` stays free (`lat.md/query#Temporal Syntax`). The archived capability `sparql-temporal-dataset` already reserves `SERVICE <tm:asOf/150> { GRAPH <g> { … } }`.
- **Nodes have no lifetime.** A node exists while any triple mentions it (`lat.md/data-model#Nodes and Identity`). Only statements are asserted and retracted.
- **Assert is idempotent** on a live identical `(s, p, o)` and valid-time interval; `create` always mints a new eid (D5). Retract cascades over subject and object positions (D8).
- **Never forget (D17):** nothing is deleted, so removing something from a graph is a retraction with a time and a reason, and the past view still shows it.
- **`sys:` is reserved.** User data cannot assert `sys:` predicates except schema flags, vocab, prefix settings and tx metadata (`lat.md/data-model#Reserved Namespaces`).
- **RDF 1.2 annotations** (`sparql-rdf12-annotations`) already let a query say `s p o ~ ?r {| q v |}`, so anything that is a statement can be annotated in SPARQL with no new syntax.

## Goals / Non-Goals

**Goals:**
- Standard SPARQL 1.1 dataset syntax (`GRAPH`, `FROM`, `FROM NAMED`, `WITH`, `USING`, and the common graph-management operations) works on named graphs.
- Metadata about a graph is queried and written like any other triple, and metadata about a membership (who put this statement in the graph) is a layer on the membership.
- No new table, no new column and no format change. Graph membership benefits from bitemporality and never-forget for free.
- Existing behaviour is unchanged for databases that never use `GRAPH`.

**Non-Goals:**
- A quad column, per-graph access control or per-graph physical storage.
- Cypher graph selection (`USE GRAPH`). Cypher still has one graph per database file.
- `LOAD`, `ADD`, `MOVE`, `COPY`, `CLEAR DEFAULT`, `CLEAR ALL`, and RDF dataset import or export of graph names (N-Quads, TriG). The tests already have these fixtures, but wiring them is a follow-up change.
- Federated `SERVICE`.
- Property paths inside `GRAPH` (see Open Questions).
- Graph names that are literals or statements.

## Decisions

### Decision 1: A graph is a node, membership is a layer statement

A membership is the statement `(m, e, sys:inGraph, g)` where `e` is the eid of a member statement (`s = e`) and `g` is the graph node (`o = g`). The graph node is an `IRI`, `NODE` or `BNODE` id. Anything else is rejected with `InvalidGraphName`.

Alternatives considered:
- *A `g` column on `triple` (quads).* Rejected by D3, and rejected again here. It adds 8 bytes to every row and every index (`lat.md/storage#Triple Table`), it forces the metadata-on-a-fact case (one fact in two graphs) to choose between duplicating the row and a join table, and the two-graph case then needs an extra table anyway.
- *A `graph` table of `(graph_id, statement_eid)`.* This is a membership statement without the statement machinery. It would need its own lifetime columns, its own triggers and its own history, all of which the `triple` table already provides.
- *One statement per graph occurrence (a quad-like eid per `(s, p, o, g)`).* Clean deletion, but assert's idempotency key then includes graph membership, and confidence attached to a fact would be per graph, not per fact. Kept as Alternative B in Decision 3.

Consequences: membership is bitemporal and time-travel works (`asOf` shows the graph as it was), memberships can carry layers (`{| v:addedBy … |}` in SPARQL 1.2 syntax on `sys:inGraph` needs no new grammar), and a graph exists without a special existence record. Cost: one extra 4-column row and its indexes per membership, and one extra join per `GRAPH` pattern.

### Decision 2: Graphs are tags, not containers

A statement has one eid whatever the number of graphs. It can be in none, one or many. `INSERT DATA { GRAPH <g1> { t } }` and `INSERT DATA { GRAPH <g2> { t } }` produce one statement with two memberships. There is no "graph of the statement" and no move semantics.

This matches how layers already work: a confidence `(e, v:confidence, 0.8)` belongs to the fact, not to the graph. It also keeps `assert` idempotent on `(s, p, o)` (D5) and leaves `pred_multi` and duplicate handling alone (`lat.md/storage#Multi-Eid Predicates`).

### Decision 3: Delete from a graph removes membership only

- `DELETE DATA { GRAPH <g> { t } }`, `DELETE { GRAPH <g> { … } } WHERE …` and `Tx::remove_from_graph` retract the membership statement. The member statement stays live and remains in every other graph and in the default graph.
- A plain `DELETE DATA { t }` retracts the statement, and the cascade retracts all its memberships along with every other layer, as today (D8).
- `CLEAR GRAPH <g>` and `DROP GRAPH <g>` retract all live memberships in `g`. They never retract a member statement.

The default graph is the union of everything (Decision 4), so "removing from the graph" cannot mean "removing from the store" without also removing the statement from its other graphs, and there is no marker for "also asserted outside any graph" to make that safe.

*Alternative B (rejected, recorded for reversal):* separate eids per `(s, p, o, graph)`, so a delete in `g` retracts exactly the statement in `g`. It gives W3C quad semantics, including deletion, but `assert` idempotency, `sys:unique`, `sys:cardinality` and the duplicate-removal logic would all need a graph argument. It also splits metadata on a fact across copies. If users need "CLEAR GRAPH removes the data", the recommended path is an explicit `Tx::retract_graph_members(g)` that cascades over members, not a change of model.

Deviation from W3C SPARQL Update: `CLEAR GRAPH <g>` does not remove triples from the store. The spec lists this so a W3C test-suite allow-list can name it.

### Decision 4: Default graph is the union

With no `FROM`, the default graph is every visible statement, in a graph or not. This is what users get today, so no existing query changes. With `FROM <g1> FROM <g2>`, the default graph is the set of statements that are members of g1 or g2. A statement in both appears once, because it is one statement. `FROM NAMED <g>` restricts what `GRAPH ?g` ranges over. `GRAPH <g> { … }` selects members of `g`, whether or not `g` was listed in `FROM NAMED`, when no `FROM NAMED` is present, and only listed graphs when one is.

Deviation from W3C SPARQL 1.1, where the default graph contains only triples outside named graphs. The union default matches Oxigraph's `default_graph_as_union` option, and it is what a memory query wants: recall should not depend on which session wrote the fact. A `FILTER NOT EXISTS { ?e sys:inGraph ?any }` idiom selects statements that are in no graph.

### Decision 5: Membership is read in the pattern's view

For a pattern evaluated in view V (transaction time and optional valid time, `lat.md/query#Views and Scans`), a statement is a member of `g` in V if the statement is visible in V and a membership `(e, sys:inGraph, g)` is visible in V. Both use the same view predicate function from M1. Under `asOf`, the graph is as it was; under `History`, every historical membership is a row; under `validAt`, membership's own valid time is honoured (a membership with `v_from`/`v_to` gives "was in this graph from March to June"). Membership inserted through SPARQL has unbounded valid time, as every SPARQL insert does today.

This is the only new time rule, and it is not new code: it is the join that `GRAPH` lowers to.

### Decision 6: Lowering is a join, not a new operator

`GRAPH <g> { P }` lowers to the patterns of `P`, each with a graph selector, and the planner emits for every selected pattern an extra join `triple AS m ON m.s = t.eid AND m.p = :inGraph AND m.o = :g` under the same view predicate. `GRAPH ?g` binds `m.o`. `FROM` and `FROM NAMED` make the selector a set of ids. The graph selector belongs on `TriplePattern` in `tm-ir`, so Cypher could use it later.

`GRAPH <g>` seeks `(p = inGraph, o = g)` on the live `(p, o, …)` index and joins to the member row by rowid, so cost is the size of the graph, not of the store. Planner statistics must know `sys:inGraph` as a predicate with a large, skewed `o` distribution; see Risks.

`GRAPH ?g` over a set: one row per `(statement, graph)` membership. A statement in two graphs gives two rows.

### Decision 7: The engine writes `sys:inGraph`; graph names are checked

`sys:inGraph` is not assertable by users through `INSERT DATA { e sys:inGraph g }`; that fails with `ReservedNamespace` like other engine predicates. Only `GRAPH` blocks in updates and the `Tx` methods create memberships. That gives one place to check the graph name (`InvalidGraphName` for a literal or a statement), to make membership idempotent, and to refuse memberships on `sys:` statements (their predicate is reserved and their meaning is engine bookkeeping, so they belong to no user graph).

`sys:inGraph` is readable in SPARQL like other `sys:` triples, so `SELECT ?g WHERE { ?e sys:inGraph ?g }` works. It is hidden from Cypher `keys()`, `properties()` and `labels()` by the existing rule.

### Decision 8: Graph metadata needs no declaration

`(v:session12 v:startedBy v:agent7)` is a statement about a node that may or may not be a graph. `GRAPH ?g` lists nodes that are objects of at least one visible membership, so an empty graph is not listed. `CREATE GRAPH <g>` (and `Tx::create_graph`) asserts `(g, rdf:type, sys:Graph)` idempotently, and `?g a sys:Graph` lists declared graphs, including empty ones. `DROP GRAPH <g>` clears memberships and retracts that declaration. It does not retract other metadata on `g`, because dropping a container should not silently erase what was recorded about it, and never-forget applies anyway; a user who wants that writes an ordinary `DELETE`.

### Decision 9: Naming

`sys:inGraph` is the membership predicate and `sys:Graph` the declaration class, both in the reserved `sys:` namespace. The illustrative `sys:inContext` in `lat.md/data-model#Layers` is replaced by `sys:inGraph` when this is implemented. Context-like groupings that are not RDF graphs (session, agent) remain plain layer triples, which is the pattern D3 describes; named graphs are for users who want RDF dataset syntax.

## Risks / Trade-offs

- **Membership rows dominate storage when graphs are used heavily.** A statement in one graph costs about two rows instead of one. Measure on the 11M-statement fixture (`bench/engine-comparison/`) and record the ratio in the change's benchmark task. If it matters, the mitigation is an optional sidecar table for single-graph memberships behind the same API. It is not planned.
- **Measured (11 M statements, `bench/named-graphs`):** one membership per live statement grows the file 1.92 times (3.38 GB against 1.76 GB). `GRAPH <100 members>` takes 0.01 ms, a 50 000-member graph 49 ms, a 5 M-member graph 550 ms, `GRAPH ?g` over all live statements 7.7 s. The seek uses `hist_pos` or `live_pos`, so no format change is needed.
- **Join order and statistics.** `sys:inGraph` rows have very uneven `o` (a graph with 10⁶ members next to one with 3). SQLite needs `sqlite_stat4` to pick the small side. Tasks include a check that `ANALYZE` covers it and a plan test for `GRAPH <small>` over a large store, per D19.
- **Index coverage.** The design expects the live `(p, o, …)` partial index to serve `GRAPH <g>`. Task 1.2 verifies this with `EXPLAIN QUERY PLAN`; if a covering index is needed it is a format-version bump and a decision for M0's owner, not a silent change.
- **W3C conformance deviations.** Union default graph (Decision 4) and membership-only deletion (Decision 3) will fail some W3C update and dataset tests. They belong on the allow-list with a reason each, in oxilite's style (`D23`).
- **Idempotent membership races** are not possible: there is a single writer (`lat.md/architecture#Connections and Concurrency`).
- **Complexity of two ways to say "context".** Users may use `sys:inGraph` and their own `v:session` layer for the same purpose. The docs must say when to use which: use graphs for dataset syntax and interchange; use plain layers when a query never needs `GRAPH`.

## Migration Plan

Additive. A database that never uses `GRAPH` has no `sys:inGraph` rows and behaves as before. There is no format change and no data migration. Queries that used to fail with `Unsupported("named graph")` now run. Rolling back means removing the feature; existing memberships stay as inert `sys:` rows and can be found with `?e sys:inGraph ?g`.

## Open Questions

- **Paths inside `GRAPH`.** The path engine (`add-path-engine`) takes a `View`, not a graph selector. This change specifies `Unsupported { feature: "named graph path" }` for property paths, `shortestPath` patterns and `tm_path` calls under `GRAPH` or a `FROM <g>` default graph. A follow-up adds a graph filter to the neighbour fetcher.
- **Should `GRAPH` accept statements as graph names?** A statement can already be described by layers, so a "graph" that is a reified fact is possible in principle. Rejected for now with `InvalidGraphName`.
- **RDF import and export of graph names** (N-Quads, TriG) is a natural next change once this lands.
- **Alternative B** (Decision 3) should be revisited if users report that `CLEAR GRAPH` not deleting data is surprising.
