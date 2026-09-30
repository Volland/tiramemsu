## Purpose

Defines named graphs on Tiramemsu's addressable statements. A graph is a node, membership is a layer statement `(eid, sys:inGraph, g)`, and metadata about a graph is ordinary triples about that node. SPARQL `GRAPH`, `FROM`, `FROM NAMED` and updates work on graphs, and time travel applies to membership like any other statement.

## ADDED Requirements

### Requirement: Graph and membership model

A named graph SHALL be a node whose id is an `IRI`, `NODE` or `BNODE`. A statement `e` SHALL be a member of graph `g` in a view when `e` is visible in that view and a membership statement `(m, e, sys:inGraph, g)` is visible in the same view. A membership SHALL be a statement with its own eid, lifetime and valid time, and it SHALL accept layers. A statement SHALL be a member of any number of graphs, or none, and SHALL keep one eid whatever the number. A graph SHALL NOT need a declaration to exist: it exists in a view while at least one membership in it is visible. The storage format, the ObjectId encoding and the `triple` table SHALL NOT change.

#### Scenario: One statement in two graphs
- **WHEN** `INSERT DATA { GRAPH <g1> { v:alice v:worksAt v:acme } }` and then `INSERT DATA { GRAPH <g2> { v:alice v:worksAt v:acme } }` are submitted
- **THEN** exactly one live statement `(v:alice v:worksAt v:acme)` exists, and it has two live `sys:inGraph` memberships, one in `g1` and one in `g2`

#### Scenario: Statement in no graph
- **WHEN** `INSERT DATA { v:alice v:name "Alice" }` is submitted
- **THEN** the statement is live and has no membership, and `SELECT ?g WHERE { GRAPH ?g { v:alice v:name "Alice" } }` returns no rows

#### Scenario: Membership is a statement with layers
- **WHEN** `(v:alice v:worksAt v:acme)` is in `<g1>` and `INSERT { ?m v:addedBy v:agent7 } WHERE { ?e sys:inGraph <g1> ~ ?m }` is submitted
- **THEN** `SELECT ?who WHERE { ?e sys:inGraph <g1> ~ ?m {| v:addedBy ?who |} }` returns one row with `who = v:agent7`

### Requirement: Graph names

A graph name SHALL be an IRI, or a blank node or anonymous node id. A `GRAPH` block, `FROM`, `FROM NAMED`, `WITH`, `USING`, graph-management operation or `Tx` graph method that names a literal, a statement eid, a transaction or a `tm:` time IRI as a graph SHALL fail: a literal, statement or transaction with `InvalidGraphName { term }`, and a `tm:` IRI with a `Parse` error that names `SERVICE`, as the temporal dataset capability specifies. Nothing SHALL be written or read in either case.

#### Scenario: Literal as graph name
- **WHEN** `Tx::add_to_graph(e1, Literal("g"))` is called
- **THEN** it fails with `InvalidGraphName` and no membership is created

#### Scenario: Statement as graph name
- **WHEN** `Tx::add_to_graph(e1, e2)` is called with `e2` a statement eid
- **THEN** it fails with `InvalidGraphName`

### Requirement: Graph metadata is ordinary triples

Triples whose subject is a graph node SHALL be ordinary statements. They SHALL NOT be members of the graph merely because their subject is the graph, and they SHALL be visible in the default graph. A query SHALL read and write graph metadata with the same patterns and updates as any other triple, with no special syntax.

#### Scenario: Metadata read and written like any triple
- **WHEN** `INSERT DATA { v:session12 v:startedBy v:agent7 ; v:startedAt "2026-09-30T09:00:00Z"^^xsd:dateTime }` and `SELECT ?g ?a WHERE { ?g v:startedBy ?a }` are run
- **THEN** one row `g = v:session12`, `a = v:agent7` is returned

#### Scenario: Metadata is not membership
- **WHEN** `(v:session12 v:startedBy v:agent7)` is live and `(v:alice v:name "Alice")` is in `v:session12`, and `SELECT ?s WHERE { GRAPH v:session12 { ?s ?p ?o } }` is run
- **THEN** only `v:alice` is returned, not `v:session12`

### Requirement: Default graph is the union

With no `FROM` clause and no `USING`, the default graph SHALL be every statement visible in the view, whether it is in a graph or not. Queries that never use graphs SHALL return the same results as before this capability. A statement that is in several graphs SHALL appear once in the default graph.

#### Scenario: Default graph sees everything
- **WHEN** `(v:a v:p v:b)` is in `<g1>` and `(v:c v:p v:d)` is in no graph, and `SELECT ?s WHERE { ?s v:p ?o }` is run
- **THEN** two rows, `v:a` and `v:c`, are returned

#### Scenario: Statement in two graphs appears once
- **WHEN** `(v:a v:p v:b)` is in `<g1>` and `<g2>` and `SELECT ?s WHERE { ?s v:p ?o }` is run
- **THEN** one row is returned

#### Scenario: Statements in no graph are selectable
- **WHEN** `SELECT ?s WHERE { ?e sys:subject ?s . FILTER NOT EXISTS { ?e sys:inGraph ?g } }` is run over the store above
- **THEN** only `v:c` is returned

### Requirement: GRAPH selects by membership

`GRAPH <g> { P }` SHALL evaluate `P` over the statements that are members of `g` in the pattern's view. `GRAPH ?g { P }` SHALL bind `?g` to each graph in which the matched statement is a member, and SHALL yield one solution per (solution of `P`, membership). `?g` bound before the block SHALL restrict it to that graph. A `GRAPH` block inside another `GRAPH` block SHALL replace the outer graph. A `GRAPH` block SHALL apply to triple patterns and to layer patterns (`~ ?r`, `{| … |}`), and the reifier or annotation triples SHALL be evaluated over the same graph selection as the statement they annotate unless they are written outside the block.

#### Scenario: GRAPH with a constant graph
- **WHEN** `(v:a v:p v:b)` is in `<g1>`, `(v:c v:p v:d)` is in `<g2>` and `SELECT ?s WHERE { GRAPH <g1> { ?s v:p ?o } }` is run
- **THEN** one row with `s = v:a` is returned

#### Scenario: GRAPH with a variable
- **WHEN** the same data is queried with `SELECT ?g ?s WHERE { GRAPH ?g { ?s v:p ?o } } ORDER BY ?g`
- **THEN** two rows are returned: `(g1, v:a)` and `(g2, v:c)`

#### Scenario: One row per membership
- **WHEN** `(v:a v:p v:b)` is in `<g1>` and `<g2>` and `SELECT ?g WHERE { GRAPH ?g { v:a v:p v:b } }` is run
- **THEN** two rows, `g1` and `g2`, are returned

#### Scenario: Bound graph variable restricts
- **WHEN** `SELECT ?s WHERE { VALUES ?g { <g2> } GRAPH ?g { ?s v:p ?o } }` is run over the two-graph data
- **THEN** one row with `s = v:c` is returned

#### Scenario: Unknown graph matches nothing
- **WHEN** `SELECT * WHERE { GRAPH <urn:never:used> { ?s ?p ?o } }` is run
- **THEN** zero rows are returned and no error is raised

### Requirement: Dataset clauses

`FROM <g1> … FROM <gn>` with non-`tm:` IRIs SHALL make the default graph the statements that are members of at least one listed graph. `FROM NAMED <g1> … <gm>` with non-`tm:` IRIs SHALL restrict `GRAPH` to the listed graphs, for both constant and variable graph names. A `tm:` time IRI SHALL keep its meaning in `FROM` and `FROM NAMED` (whole-query time) and MAY appear together with graph IRIs. `USING` and `USING NAMED` in updates SHALL mean the same for the `WHERE` pattern. A `GRAPH <g>` block naming a graph that is not listed in a `FROM NAMED` present in the query SHALL match nothing.

#### Scenario: FROM restricts the default graph
- **WHEN** `(v:a v:p v:b)` is in `<g1>`, `(v:c v:p v:d)` is in no graph, and `SELECT ?s FROM <g1> WHERE { ?s v:p ?o }` is run
- **THEN** one row with `s = v:a` is returned

#### Scenario: FROM NAMED restricts GRAPH
- **WHEN** statements are in `<g1>` and `<g2>` and `SELECT ?g FROM NAMED <g1> WHERE { GRAPH ?g { ?s ?p ?o } }` is run
- **THEN** only `g1` is returned

#### Scenario: Time IRI beside a graph IRI
- **WHEN** `(v:a v:p v:b)` was added to `<g1>` in tx 5 and removed from it in tx 9, and `SELECT ?s FROM <urn:tiramemsu:tm:asOf/7> FROM <g1> WHERE { ?s v:p ?o }` is run
- **THEN** one row with `s = v:a` is returned, and the same query with `asOf/10` returns no rows

### Requirement: Graphs combine with SERVICE time scopes

`SERVICE <tm:…> { GRAPH <g> { … } }` SHALL evaluate the `GRAPH` block in the view selected by the `SERVICE` group, and `GRAPH <g> { SERVICE <tm:…> { … } }` SHALL scope the inner group to that view and to the graph `g`. A `tm:` IRI as the name of a `GRAPH` block SHALL remain a `Parse` error naming `SERVICE`. The membership and the member statement SHALL both be read in the scoped view.

#### Scenario: Graph inside a time scope
- **WHEN** `(v:a v:p v:b)` was added to `<g1>` in tx 5 and removed from it in tx 9, and `SELECT ?s WHERE { SERVICE <urn:tiramemsu:tm:asOf/7> { GRAPH <g1> { ?s v:p ?o } } }` is run
- **THEN** one row with `s = v:a` is returned

#### Scenario: Time IRI as graph name still rejected
- **WHEN** `SELECT * WHERE { GRAPH <urn:tiramemsu:tm:asOf/7> { ?s ?p ?o } }` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL whose message names `SERVICE`

### Requirement: Membership is bitemporal

A membership SHALL have transaction time `[t_add, t_ret)` and a valid-time interval like any statement. Views SHALL apply to it: `asOf` shows membership as it was, `validAt` keeps only memberships whose valid time contains the instant, and `History` returns every membership ever stored. Memberships asserted through SPARQL SHALL have unbounded valid time. The `Tx` method `add_to_graph` MAY take a valid-time interval. Retracting a member statement SHALL retract its memberships through the cascade, with `ret_kind` cascade, in the same transaction.

#### Scenario: Past membership under asOf
- **WHEN** `(v:a v:p v:b)` was added to `<g1>` in tx 5, removed from `<g1>` in tx 9, and `ASK FROM <urn:tiramemsu:tm:asOf/7> { GRAPH <g1> { v:a v:p v:b } }` is run
- **THEN** the answer is `true`, and with `asOf/9` it is `false`

#### Scenario: Valid-time membership
- **WHEN** `Tx::add_to_graph(e1, g1, valid [2025-01-01, 2025-07-01))` was called and `ASK FROM <urn:tiramemsu:tm:validAt/2025-08-01> { GRAPH <g1> { v:a v:p v:b } }` is run
- **THEN** the answer is `false`, and with `validAt/2025-03-01` it is `true`

#### Scenario: History shows removed memberships
- **WHEN** a membership was added and later removed and `SELECT ?g FROM <urn:tiramemsu:tm:history> WHERE { GRAPH ?g { v:a v:p v:b } }` is run
- **THEN** one row with the graph is returned

#### Scenario: Cascade retracts memberships
- **WHEN** `(v:a v:p v:b)` is in `<g1>` and `DELETE DATA { v:a v:p v:b }` is submitted
- **THEN** the statement and its membership are both retracted in the same transaction, and the membership has `ret_kind` cascade

### Requirement: Adding to a graph

`INSERT DATA { GRAPH <g> { t … } }` and a template `GRAPH <g> { t … }` in `INSERT` SHALL assert each triple `t` idempotently under the existing assert rules (schema flags, `sys:` reservation, self-reference rule) and SHALL assert a membership `(t, sys:inGraph, g)` idempotently. A triple that is already live and in `g` SHALL produce no new statement and no new membership. A triple that is already live but not in `g` SHALL keep its eid and gain one membership. The update report SHALL list new statements and new memberships separately. `WITH <g>` SHALL make `<g>` the graph of every template triple that is not inside its own `GRAPH` block, and SHALL scope the `WHERE` pattern's default graph to `<g>` as `USING <g>` does. A `GRAPH` block in `INSERT DATA`, `DELETE DATA` or a template with a graph name that is a variable SHALL be allowed only in a template whose `WHERE` binds it.

#### Scenario: Insert into a new graph
- **WHEN** `INSERT DATA { GRAPH <g1> { v:a v:p v:b } }` is submitted on an empty store
- **THEN** the statement and one membership in `g1` are live, and the report lists one statement and one membership

#### Scenario: Insert into an already-live statement
- **WHEN** `(v:a v:p v:b)` is live with eid e1 and in no graph, and `INSERT DATA { GRAPH <g1> { v:a v:p v:b } }` is submitted
- **THEN** e1 stays the only statement, gains one membership in `g1`, and the report lists no new statement and one new membership

#### Scenario: Insert is idempotent
- **WHEN** the same `INSERT DATA { GRAPH <g1> { v:a v:p v:b } }` is submitted twice
- **THEN** the second request creates no statement and no membership

#### Scenario: WITH sets the graph of a template
- **WHEN** `WITH <g1> INSERT { v:a v:p v:b } WHERE {}` is submitted
- **THEN** `(v:a v:p v:b)` is live and a member of `g1`

#### Scenario: Graph variable bound by WHERE
- **WHEN** `INSERT { GRAPH ?g { v:audit v:saw ?s } } WHERE { GRAPH ?g { ?s v:p ?o } }` is submitted over data in `<g1>` and `<g2>`
- **THEN** a statement `(v:audit v:saw v:a)` is in `g1`, and `(v:audit v:saw v:c)` is in `g2`

### Requirement: Deleting from a graph

`DELETE DATA { GRAPH <g> { t … } }` and a template `GRAPH <g> { t … }` in `DELETE` SHALL retract the membership `(t, sys:inGraph, g)` and SHALL NOT retract the statement `t`. A triple that is not a member of `g` SHALL be a no-op and SHALL NOT be an error. A delete without a `GRAPH` block SHALL retract the statement with the cascade of its layers, and therefore its memberships in every graph. The update report SHALL list retracted memberships separately from retracted statements.

#### Scenario: Removing from one graph keeps the statement
- **WHEN** `(v:a v:p v:b)` is in `<g1>` and `<g2>` and `DELETE DATA { GRAPH <g1> { v:a v:p v:b } }` is submitted
- **THEN** the membership in `g1` is retracted, and the statement and its membership in `g2` stay live

#### Scenario: Removing from a graph the statement is not in
- **WHEN** `(v:a v:p v:b)` is in `<g1>` only and `DELETE DATA { GRAPH <g2> { v:a v:p v:b } }` is submitted
- **THEN** nothing is retracted and no error is raised

#### Scenario: Plain delete removes every membership
- **WHEN** `(v:a v:p v:b)` is in `<g1>` and `<g2>` and `DELETE DATA { v:a v:p v:b }` is submitted
- **THEN** the statement and both memberships are retracted

### Requirement: Graph management operations

`CREATE GRAPH <g>` SHALL assert `(g, rdf:type, sys:Graph)` idempotently. `CLEAR GRAPH <g>` and `CLEAR NAMED` SHALL retract every live membership in the named graph, or in every graph, and SHALL NOT retract any member statement. `DROP GRAPH <g>` and `DROP NAMED` SHALL do the same as `CLEAR` and SHALL also retract the `(g, rdf:type, sys:Graph)` declaration where it exists, and SHALL NOT retract other triples with `g` as subject. `SILENT` SHALL make a missing graph a no-op. Without `SILENT`, `CLEAR GRAPH`, `DROP GRAPH` on a graph that has neither a live membership nor a declaration SHALL fail with `GraphNotFound { graph }`, and `CREATE GRAPH` on a declared graph SHALL fail with `GraphExists { graph }`. `LOAD`, `ADD`, `MOVE`, `COPY`, `CLEAR DEFAULT`, `CLEAR ALL`, `DROP DEFAULT` and `DROP ALL` SHALL still fail with `Unsupported` naming the operation before anything is written.

#### Scenario: CLEAR keeps the statements
- **WHEN** `(v:a v:p v:b)` is in `<g1>` and `CLEAR GRAPH <g1>` is submitted
- **THEN** `(v:a v:p v:b)` is still live, and `SELECT * WHERE { GRAPH <g1> { ?s ?p ?o } }` returns no rows

#### Scenario: CREATE makes an empty graph visible
- **WHEN** `CREATE GRAPH <g9>` is submitted and `SELECT ?g WHERE { ?g a sys:Graph }` is run
- **THEN** one row with `g = <g9>` is returned, and `SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }` does not return `g9`

#### Scenario: DROP keeps other metadata
- **WHEN** `<g1>` has members and `(<g1> v:startedBy v:agent7)` and `DROP GRAPH <g1>` is submitted
- **THEN** the memberships and the `sys:Graph` declaration are retracted, and `(<g1> v:startedBy v:agent7)` is still live

#### Scenario: Missing graph without SILENT
- **WHEN** `CLEAR GRAPH <urn:never:used>` is submitted
- **THEN** the request fails with `GraphNotFound` naming the graph, and `CLEAR SILENT GRAPH <urn:never:used>` succeeds and retracts nothing

#### Scenario: Still-unsupported operation
- **WHEN** `COPY <g1> TO <g2>` is submitted
- **THEN** the request fails with `Unsupported { feature: "COPY" }` and nothing is written

### Requirement: Membership predicate is engine-owned

The predicate `sys:inGraph` SHALL be asserted only by `GRAPH` blocks in updates, by `WITH`, and by `Tx::add_to_graph`. An assert of a triple with the predicate `sys:inGraph` through `INSERT DATA`, an ordinary template or the plain `Tx::assert` SHALL fail with `ReservedNamespace` naming `urn:tiramemsu:sys:inGraph`. A statement whose predicate is in the `sys:` namespace SHALL NOT be a graph member: adding it to a graph SHALL fail with `ReservedNamespace`. `sys:inGraph` triples SHALL be readable in SPARQL and SHALL be hidden from Cypher `keys()`, `properties()` and `labels()` like other `sys:` triples.

#### Scenario: Direct insert of membership is rejected
- **WHEN** `INSERT DATA { <urn:tiramemsu:stmt:1> sys:inGraph <g1> }` is submitted
- **THEN** the request fails with `ReservedNamespace` naming `urn:tiramemsu:sys:inGraph` and nothing is written

#### Scenario: Schema statements cannot be graph members
- **WHEN** `INSERT DATA { GRAPH <g1> { v:email sys:unique true } }` is submitted
- **THEN** the request fails with `ReservedNamespace` and nothing is written

#### Scenario: Membership is readable
- **WHEN** `(v:a v:p v:b)` is in `<g1>` and `SELECT ?g WHERE { ?e sys:inGraph ?g }` is run
- **THEN** one row with `g = <g1>` is returned

### Requirement: Membership is subject to predicate schema

Predicate schema flags SHALL apply to the member statement as they do outside graphs (`sys:valueType`, `sys:unique`, `sys:cardinality`). A supersede of a member statement by cardinality one or by `Tx::supersede` SHALL retract the old statement and its memberships through the cascade, and SHALL NOT copy memberships to the replacement. Adding the replacement to graphs SHALL be an explicit act of the writer.

#### Scenario: Cardinality one drops the old memberships
- **WHEN** `v:age` has `sys:cardinality sys:one`, `(v:alice v:age 41)` is in `<g1>`, and `INSERT DATA { GRAPH <g1> { v:alice v:age 42 } }` is submitted
- **THEN** the old statement and its membership are retracted with kind cardinality, and `(v:alice v:age 42)` is live in `g1`

### Requirement: Rust API

The facade SHALL provide on `Tx`: `add_to_graph(eid, graph, opts)` (idempotent, returns the membership eid and whether it is new), `remove_from_graph(eid, graph)` (retracts the membership, returns whether one was live), `clear_graph(graph)` (retracts all live memberships in the graph and returns their eids), and `create_graph(graph)`. It SHALL provide on `View`: `graphs()` (the graphs with at least one visible membership or a declaration) and `graph_members(graph)` (member statement eids visible in the view). All of them SHALL reject a non-node graph with `InvalidGraphName`, and all of them SHALL respect the view's transaction time and valid time. The MCP write tool SHALL accept an optional `graph`, and the MCP search tool SHALL accept an optional `graph` filter.

#### Scenario: Add and read back
- **WHEN** `tx.add_to_graph(e1, g1, default)` is called and committed and `view.graph_members(g1)` is called on the now view
- **THEN** it returns `[e1]`, and `view.graphs()` returns `[g1]`

#### Scenario: Idempotent add
- **WHEN** `add_to_graph(e1, g1)` is called twice in one transaction
- **THEN** both calls return the same membership eid, the second reporting `new = false`

#### Scenario: Time travel through the API
- **WHEN** `e1` was added to `g1` in tx 5 and removed in tx 9, and `db.as_of(Tx(7)).graph_members(g1)` is called
- **THEN** it returns `[e1]`, and `db.as_of(Tx(9)).graph_members(g1)` returns `[]`

### Requirement: Errors

The facade error set SHALL gain `InvalidGraphName { term }`, `GraphNotFound { graph }` and `GraphExists { graph }`. A failed graph operation SHALL fail the whole request or transaction and write nothing, like every other update failure.

#### Scenario: Atomic failure
- **WHEN** `INSERT DATA { v:a v:p v:b } ; INSERT DATA { GRAPH "not-an-iri" { v:c v:p v:d } }` is submitted
- **THEN** the request fails with `InvalidGraphName` and `(v:a v:p v:b)` is not stored

### Requirement: Unsupported combinations fail before execution

A property path, a `shortestPath` or `allShortestPaths` pattern, or a `tm_path` call inside a `GRAPH` block or under a `FROM <g>` default graph SHALL fail with `Unsupported { feature: "named graph path" }` until the path engine supports a graph filter. Cypher SHALL keep its single graph: a Cypher `USE` clause naming a graph SHALL fail with `Unsupported { feature: "USE GRAPH" }`.

#### Scenario: Path inside GRAPH
- **WHEN** `SELECT ?x WHERE { GRAPH <g1> { v:a v:knows+ ?x } }` is submitted
- **THEN** the request fails with `Unsupported { feature: "named graph path" }`

#### Scenario: Cypher USE GRAPH
- **WHEN** `USE GRAPH g1 MATCH (n) RETURN n` is submitted through Cypher
- **THEN** the request fails with `Unsupported { feature: "USE GRAPH" }`
