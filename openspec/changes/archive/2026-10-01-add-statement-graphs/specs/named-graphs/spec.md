## MODIFIED Requirements

### Requirement: Graph and membership model

A named graph SHALL be a node or a statement: its id is an `IRI`, `NODE`, `BNODE` or `STMT`. A statement `e` SHALL be a member of graph `g` in a view when `e` is visible in that view and a membership statement `(m, e, sys:inGraph, g)` is visible in the same view. A membership SHALL be a statement with its own eid, lifetime and valid time, and it SHALL accept layers. A statement SHALL be a member of any number of graphs, or none, and SHALL keep one eid whatever the number. A graph SHALL NOT need a declaration to exist: it exists in a view while at least one membership in it is visible. The storage format, the ObjectId encoding and the `triple` table SHALL NOT change.

#### Scenario: One statement in two graphs
- **WHEN** `INSERT DATA { GRAPH <g1> { v:alice v:worksAt v:acme } }` and then `INSERT DATA { GRAPH <g2> { v:alice v:worksAt v:acme } }` are submitted
- **THEN** exactly one live statement `(v:alice v:worksAt v:acme)` exists, and it has two live `sys:inGraph` memberships, one in `g1` and one in `g2`

#### Scenario: Statement in no graph
- **WHEN** `INSERT DATA { v:alice v:name "Alice" }` is submitted
- **THEN** the statement is live and has no membership, and `SELECT ?g WHERE { GRAPH ?g { v:alice v:name "Alice" } }` returns no rows

#### Scenario: Membership is a statement with layers
- **WHEN** `(v:alice v:worksAt v:acme)` is in `<g1>` and `INSERT { ?m v:addedBy v:agent7 } WHERE { ?e sys:inGraph <g1> ~ ?m }` is submitted
- **THEN** `SELECT ?who WHERE { ?e sys:inGraph <g1> ~ ?m {| v:addedBy ?who |} }` returns one row with `who = v:agent7`

#### Scenario: An edge holds a subgraph
- **WHEN** `(v:p7 v:enrolledIn v:trial3)` is live with eid e1 and `INSERT DATA { GRAPH <urn:tiramemsu:stmt:e1> { v:drSmith v:role v:investigator } }` is submitted
- **THEN** `SELECT ?s WHERE { v:p7 v:enrolledIn v:trial3 ~ ?e . GRAPH ?e { ?s v:role ?r } }` returns one row with `s = v:drSmith`

### Requirement: Graph names

A graph name SHALL be an IRI, a blank node or anonymous node id, or a statement eid. A `GRAPH` block, `FROM`, `FROM NAMED`, `WITH`, `USING`, graph-management operation or `Tx` graph method that names a literal, a transaction or a `tm:` time IRI as a graph SHALL fail: a literal or transaction with `InvalidGraphName { term }`, and a `tm:` IRI with a `Parse` error that names `SERVICE`, as the temporal dataset capability specifies. Adding a membership to a statement-named graph SHALL require the graph statement to be live, and SHALL fail with `NotLive(graph)` otherwise. Nothing SHALL be written or read in any of these failure cases.

#### Scenario: Literal as graph name
- **WHEN** `Tx::add_to_graph(e1, Literal("g"))` is called
- **THEN** it fails with `InvalidGraphName` and no membership is created

#### Scenario: Transaction as graph name
- **WHEN** `SELECT * FROM <urn:tiramemsu:tx:1> WHERE { ?s ?p ?o }` is submitted
- **THEN** it fails with `InvalidGraphName`

#### Scenario: Statement as graph name
- **WHEN** `Tx::add_to_graph(e1, e2)` is called with `e2` a live statement eid
- **THEN** a membership `(e1 sys:inGraph e2)` is created, and `View::graph_members(e2)` returns `[e1]`

#### Scenario: Retracted statement as graph name
- **WHEN** `Tx::add_to_graph(e1, e2)` is called with `e2` a retracted statement eid
- **THEN** it fails with `NotLive(e2)` and no membership is created

### Requirement: Membership is subject to predicate schema

Predicate schema flags SHALL apply to the member statement as they do outside graphs (`sys:valueType`, `sys:unique`, `sys:cardinality`). A supersede of a member statement by cardinality one or by `Tx::supersede` SHALL retract the old statement and its memberships through the cascade, and SHALL NOT copy the old statement's own memberships to the replacement. Adding the replacement to graphs SHALL be an explicit act of the writer. A supersede of a statement that names a graph SHALL replay the memberships of that graph onto the new eid: every membership in the cascade set whose graph is in the cascade set SHALL be inserted again with its subject and graph mapped through σ. The member statements SHALL keep their eids.

#### Scenario: Cardinality one drops the old memberships
- **WHEN** `v:age` has `sys:cardinality sys:one`, `(v:alice v:age 41)` is in `<g1>`, and `INSERT DATA { GRAPH <g1> { v:alice v:age 42 } }` is submitted
- **THEN** the old statement and its membership are retracted with kind cardinality, and `(v:alice v:age 42)` is live in `g1`

#### Scenario: Supersede keeps the contents of an edge
- **WHEN** e1 `(v:p7 v:enrolledIn v:trial3)` holds e5 `(v:drSmith v:role v:investigator)` in its graph, and `Tx::supersede(e1, Patch::object(v:trial4))` returns e9
- **THEN** `View::graph_members(e9)` returns `[e5]`, e5 is still live with the same eid, and `View::graph_members(e1)` on the now view returns `[]`

### Requirement: Membership is bitemporal

A membership SHALL have transaction time `[t_add, t_ret)` and a valid-time interval like any statement. Views SHALL apply to it: `asOf` shows membership as it was, `validAt` keeps only memberships whose valid time contains the instant, and `History` returns every membership ever stored. Memberships asserted through SPARQL SHALL have unbounded valid time. The `Tx` method `add_to_graph` MAY take a valid-time interval. Retracting a member statement SHALL retract its memberships through the cascade, with `ret_kind` cascade, in the same transaction. Retracting a statement that names a graph SHALL retract every membership in that graph through the cascade in the same transaction, and SHALL NOT retract the member statements.

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

#### Scenario: Retracting an edge empties its graph and keeps the members
- **WHEN** e5 is in the graph of statement e1 and e1 is retracted in tx 9
- **THEN** the membership is retracted with `ret_kind` cascade, e5 stays live, `graph_members(e1)` is `[]` now and `[e5]` as of tx 8
