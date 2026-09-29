## Purpose

Defines how Cypher write clauses change the store. Each clause maps onto Tiramemsu's transaction operations (create, assert, upsert, supersede, retract with cascade), and each query is atomic.

## ADDED Requirements

### Requirement: Write entry points and atomicity
The system SHALL execute Cypher queries that contain write clauses through a transaction handle with the signature `cypher(query, params)`, and through a database-level convenience with the signature `cypher_write(options, query, params)` that opens exactly one transaction. The whole query SHALL run in one transaction on the single writer with one transaction number `t`. It SHALL return the `RETURN` rows together with the transaction report (asserted, existing, retracted and superseded eids). If any clause fails, the query MUST fail, and the transaction MUST leave no trace: no tx row, statements or terms. Clauses SHALL apply in order, and every clause SHALL see the effects of the earlier clauses in the same query.

#### Scenario: Create and return in one transaction
- **WHEN** `CREATE (n:Person {name: 'Bob'}) RETURN n.name AS name` runs through the write entry point
- **THEN** the result has one row `"Bob"`
- **AND** the report lists the two new eids (the `rdf:type` and `v:name` statements) under one transaction number

#### Scenario: Later clauses see earlier writes
- **WHEN** `CREATE (n:Temp {k: 1}) WITH n MATCH (m:Temp) RETURN count(m) AS c` runs on a store with no `Temp` nodes
- **THEN** `c` is 1

#### Scenario: Failure leaves no trace
- **WHEN** `v:email` is `sys:unique`, `(v:alice v:email "a@x")` is live, and `CREATE (n:Person {name: 'Eve'}) CREATE (m {email: 'a@x'})` runs
- **THEN** the query fails with `UniqueViolation`
- **AND** no `Eve` node exists and the last transaction number is unchanged

#### Scenario: Transaction handle composes with API operations
- **WHEN** inside one `transact` call the program asserts `(v:alice v:age 42)` through the API and then runs `MATCH (n) WHERE n.age = 42 SET n:Adult` through the transaction handle
- **THEN** both changes commit under the same `t`

### Requirement: CREATE nodes
`CREATE` of a node pattern SHALL allocate a new anonymous node id, assert one `rdf:type` statement per label and one statement per inline property, and bind the variable to the new node. A node pattern whose property map contains the reserved key `` `@id` `` SHALL use that IRI as the node identity (see `vocabulary-mapping`), and SHALL assert its labels and properties idempotently on it. A `CREATE (n)` with no labels, properties or relationships SHALL return a node, but it SHALL write no statement, so the node is not visible to later queries; this is a documented difference from Neo4j.

#### Scenario: Anonymous node with label and properties
- **WHEN** `CREATE (n:Person {name: 'Bob', age: 30}) RETURN elementId(n) AS id` runs
- **THEN** `id` is `urn:tiramemsu:node:<k>` for a newly allocated `k`
- **AND** three statements with that subject are live: `rdf:type v:Person`, `v:name "Bob"` and `v:age 30`

#### Scenario: Node with an explicit IRI
- **WHEN** `CREATE (n:Person {`@id`: 'v:carol', name: 'Carol'})` runs
- **THEN** `(v:carol rdf:type v:Person)` and `(v:carol v:name "Carol")` are live
- **AND** no statement with predicate `@id` exists

#### Scenario: Empty node leaves no statement
- **WHEN** `CREATE (n) RETURN n` runs, followed by `MATCH (n) RETURN count(n) AS c` on an otherwise empty store
- **THEN** the first query returns one node, the second returns `c = 0`, and the transaction report lists no asserted eids

### Requirement: CREATE relationships
`CREATE` of a relationship pattern SHALL always insert a new statement with a new eid, even when an identical live statement exists, so that repeated creation yields parallel relationships. Inline relationship properties SHALL be asserted as statements whose subject is the new eid. The reserved keys `validFrom` and `validTo` in a relationship property map SHALL instead set the valid-time interval of the created statement, and they MUST be date or datetime values.

#### Scenario: Parallel edges
- **WHEN** `MATCH (a {name:'Alice'}), (b {name:'Bob'}) CREATE (a)-[:CALLED]->(b)` runs twice
- **THEN** two live statements `(alice CALLED bob)` exist with different eids
- **AND** `MATCH ({name:'Alice'})-[r:CALLED]->() RETURN count(r)` returns 2

#### Scenario: Relationship properties are layer statements
- **WHEN** `MATCH (a {name:'Alice'}), (c {name:'Acme'}) CREATE (a)-[r:worksAt {confidence: 0.8}]->(c) RETURN r` runs and the new eid is e
- **THEN** `(e v:confidence 0.8)` is live

#### Scenario: Valid time on creation
- **WHEN** `MATCH (a {name:'Alice'}), (c {name:'Acme'}) CREATE (a)-[r:worksAt {validFrom: date('2025-01-01')}]->(c) RETURN r.validFrom AS f` runs
- **THEN** `f` is DateTime 2025-01-01T00:00:00.000Z, and the statement's `v_from` is that instant
- **AND** no `v:validFrom` statement is asserted

#### Scenario: Path creation
- **WHEN** `CREATE (a:Person {name:'X'})-[:knows]->(b:Person {name:'Y'})-[:knows]->(a)` runs
- **THEN** two new nodes and two `knows` relationships between them, in opposite directions, are live

### Requirement: Property value encoding on write
The system SHALL encode written Cypher values canonically: an Integer within the 60-bit range as an inline integer (and otherwise as an `xsd:integer` typed literal), Float as a double, String as a string, Boolean as a boolean, Date as a date, and DateTime (zoned or local) as a UTC datetime. A list value SHALL be written as one statement per distinct element. An empty list or `null` SHALL write nothing, and in `SET` it SHALL remove the property. A map value, a node, relationship or path value, or a list containing `null` or a nested list, MUST fail with an `Eval` error.

#### Scenario: List property becomes several statements
- **WHEN** `CREATE (n:Doc {tags: ['a', 'b']}) RETURN n.tags AS t` runs
- **THEN** two statements `v:tags "a"` and `v:tags "b"` are live for the node, and `t` is `["a", "b"]`

#### Scenario: Map value rejected
- **WHEN** `CREATE (n {meta: {x: 1}})` runs
- **THEN** it fails with an `Eval` error and nothing is written

#### Scenario: Zoned datetime normalised
- **WHEN** `CREATE (n {at: datetime('2025-03-01T10:00:00+02:00')}) RETURN n.at AS at` runs
- **THEN** `at` is DateTime 2025-03-01T08:00:00.000Z

### Requirement: MERGE with a unique key
When a `MERGE` node pattern's property map contains a key whose resolved predicate is flagged `sys:unique true`, the system SHALL look up the node by that key and value inside the writer transaction, as an upsert. If a live subject exists it SHALL bind to it, and otherwise it SHALL create a new anonymous node and assert the key statement. It SHALL then assert the pattern's labels and remaining properties idempotently on the bound node. If several unique keys appear, the first in map order SHALL be used for the lookup. `ON CREATE SET` SHALL apply only when the node was created, and `ON MATCH SET` only when it already existed.

#### Scenario: Upsert creates when absent
- **WHEN** `v:email` is `sys:unique`, no subject has email `"a@x"`, and `MERGE (n:Person {email: 'a@x'}) ON CREATE SET n.created = true RETURN n` runs
- **THEN** a new node is returned, with `rdf:type v:Person`, `v:email "a@x"` and `v:created true` live

#### Scenario: Upsert returns existing subject
- **WHEN** the same `MERGE` runs again with `ON MATCH SET n.seen = true`
- **THEN** the same node is returned, no new node is created, and `v:seen true` is asserted on it
- **AND** the report lists the email statement under existing, not asserted

#### Scenario: Existing subject without the label gains it
- **WHEN** `(v:alice v:email "al@x")` is live without any label and `MERGE (n:Person {email: 'al@x'}) RETURN n` runs
- **THEN** `v:alice` is returned, and `(v:alice rdf:type v:Person)` becomes live

### Requirement: MERGE by pattern match
When no unique key applies, `MERGE` SHALL evaluate the whole pattern against the current state inside the writer transaction. If at least one match exists, it SHALL bind one row per match. Otherwise it SHALL create the entire pattern as `CREATE` would, and bind one row. Because there is a single writer, the match-or-create SHALL be atomic with respect to concurrent writers. A `MERGE` whose pattern contains a `null` property value MUST fail with an `Eval` error.

#### Scenario: Merge node without unique key
- **WHEN** `v:name` is not unique, `MERGE (n:City {name: 'Lviv'})` runs twice
- **THEN** exactly one node with label `City` and name `"Lviv"` exists

#### Scenario: Merge relationship between bound nodes
- **WHEN** `MATCH (a {name:'Alice'}), (b {name:'Bob'}) MERGE (a)-[r:knows]->(b) RETURN r` runs twice
- **THEN** exactly one live `knows` statement from Alice to Bob exists, and both runs return the same eid

#### Scenario: Merge binds every existing match
- **WHEN** two nodes labelled `Tag` with name `"x"` already exist and `MERGE (t:Tag {name: 'x'}) RETURN count(t) AS c` runs
- **THEN** `c` is 2 and nothing is created

#### Scenario: Concurrent merges do not duplicate
- **WHEN** two threads each run `MERGE (n:City {name: 'Kyiv'})` against the same database at the same time
- **THEN** exactly one such node exists after both commit

#### Scenario: Null in MERGE rejected
- **WHEN** `MERGE (n:City {name: $nm})` runs with `nm = null`
- **THEN** it fails with an `Eval` error

### Requirement: SET property
`SET x.key = value` SHALL consider the live property statements `(x, <resolved key>, literal)` at transaction time `Now`, regardless of valid time, and apply the first rule that fits:
1. If `value` is `null` or an empty list, retract all of them.
2. If `value` is a list, retract those whose object is not in the list, and assert each element.
3. If none exist, assert `(x, key, value)`.
4. If one of them already has object `value`, keep it (an idempotent assert) and retract the others.
5. If the predicate is `sys:cardinality sys:one`, assert `(x, key, value)`, which retracts the old values by cardinality replacement without carrying their annotations.
6. If exactly one exists, supersede it with the object patched to `value`, which replays its annotations onto the new eid.
7. Otherwise, retract all of them and assert `(x, key, value)`.

`x` may be a node, a relationship or a statement node.

#### Scenario: First value is asserted
- **WHEN** `v:alice` has no age and `MATCH (n {name:'Alice'}) SET n.age = 42` runs
- **THEN** `(v:alice v:age 42)` is live, and the report lists it under asserted

#### Scenario: Same value is a no-op
- **WHEN** `(v:alice v:age 42)` is live with eid e5 and the same `SET` runs
- **THEN** e5 is still live and listed under existing, and no statement is retracted

#### Scenario: Changing a value supersedes and keeps annotations
- **WHEN** `(v:alice v:title "Dr")` is live with eid e5, `(e5 v:source "cv")` is live, and `MATCH (n {name:'Alice'}) SET n.title = 'Prof'` runs
- **THEN** e5 and its annotation are retracted with kind `supersede`
- **AND** a new statement e9 = `(v:alice v:title "Prof")` is live with `(e9 v:source "cv")` and `(e9 sys:supersedes e5)`

#### Scenario: Cardinality-one predicate replaces without replay
- **WHEN** `v:age` is `sys:one`, `(v:alice v:age 41)` is live with annotation `(e v:source "form")`, and `SET n.age = 42` runs for alice
- **THEN** the old statement and its annotation are retracted with kind `cardinality`
- **AND** the new age statement has no `v:source` annotation

#### Scenario: Multi-valued property is replaced
- **WHEN** `v:alice` has live `v:nick "al"` and `v:nick "ally"` and `SET n.nick = 'ali'` runs
- **THEN** both old statements are retracted (kind `explicit`), and only `v:nick "ali"` is live

#### Scenario: Setting null removes
- **WHEN** `MATCH (n {name:'Alice'}) SET n.age = null` runs
- **THEN** no live `v:age` statement remains for `v:alice`

#### Scenario: SET on a relationship
- **WHEN** `MATCH (:Person {name:'Alice'})-[r:worksAt]->() SET r.confidence = 0.9` runs, and the relationship has eid e1 with no confidence
- **THEN** `(e1 v:confidence 0.9)` is live, and e1 itself is unchanged

### Requirement: SET and REMOVE with maps and labels
`SET x += map` SHALL apply `SET x.k = v` for each entry. `SET x = map` SHALL additionally retract every live, non-`sys:` property statement of `x` whose key is not in the map. `SET n:L1:L2` SHALL assert `(n rdf:type <L>)` for each label, idempotently. `REMOVE n:L` SHALL retract every live `(n rdf:type <L>)` statement. `REMOVE x.key` SHALL retract every live property statement of `x` for that key, with cascade. `SET x = <node or relationship>` MUST fail with `Unsupported`.

#### Scenario: SET += merges properties
- **WHEN** `v:alice` has name "Alice" and age 41 and `SET n += {age: 42, city: 'Lviv'}` runs
- **THEN** name "Alice", age 42 and city "Lviv" are live

#### Scenario: SET = replaces all properties
- **WHEN** the same node gets `SET n = {name: 'Alicia'}`
- **THEN** only `v:name "Alicia"` remains among its property statements, while its labels and relationships are unaffected

#### Scenario: Labels set and removed
- **WHEN** `MATCH (n {name:'Alice'}) SET n:Admin REMOVE n:Person` runs
- **THEN** `labels(n)` is `["Admin"]`

#### Scenario: REMOVE cascades annotations
- **WHEN** `(v:alice v:age 42)` is e5 with annotation `(e5 v:source "form")` and `MATCH (n {name:'Alice'}) REMOVE n.age` runs
- **THEN** e5 is retracted with kind `explicit`, and the annotation is retracted with kind `cascade` in the same transaction

### Requirement: DELETE relationships and statements
`DELETE r`, for a relationship variable or a statement node, SHALL retract that statement with cascade. Deleting an already retracted statement, a variable deleted earlier in the same query, or `null` SHALL be a no-op.

#### Scenario: Delete a relationship
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` has annotation e2 = `(e1 v:confidence 0.8)` and `MATCH (:Person {name:'Alice'})-[r:worksAt]->() DELETE r` runs
- **THEN** e1 is retracted with kind `explicit`, and e2 with kind `cascade`, both with the same `t_ret`
- **AND** a query `USE AS OF <t_ret − 1>` still returns the relationship and its confidence

#### Scenario: Delete null
- **WHEN** `OPTIONAL MATCH (n:Nobody) DELETE n` runs
- **THEN** it succeeds and retracts nothing

### Requirement: DELETE nodes without DETACH
`DELETE n` for a node SHALL retract every live statement whose subject is `n` and that is presented as a property or label, with cascade. At the end of the query, before commit, if any live statement that is presented as a relationship still has `n` as subject or object, the query MUST fail with `DeleteConnectedNode`, which names the node and lists those relationship eids. The whole transaction MUST then leave no trace. Relationships deleted by the same query SHALL NOT count.

#### Scenario: Node with relationships cannot be deleted
- **WHEN** `v:alice` has a live `worksAt` relationship and `MATCH (n {name:'Alice'}) DELETE n` runs
- **THEN** the query fails with `DeleteConnectedNode` that names `v:alice` and lists the `worksAt` eid
- **AND** alice's name statement is still live

#### Scenario: Deleting the relationships in the same query succeeds
- **WHEN** `MATCH (n {name:'Alice'})-[r]-() DELETE r, n` runs
- **THEN** it succeeds, and every property, label and relationship of `v:alice` is retracted

#### Scenario: Isolated node is deleted
- **WHEN** `v:zed` has only a label and a name and `MATCH (n {name:'Zed'}) DELETE n` runs
- **THEN** both statements are retracted, and `MATCH (n {name:'Zed'}) RETURN n` returns zero rows

### Requirement: DETACH DELETE
`DETACH DELETE n` SHALL retract, with cascade, every live statement whose subject or object is `n`: its labels, its properties, its outgoing and incoming relationships, and statements that refer to it from layers. It SHALL never fail because of connected relationships.

#### Scenario: Detach delete removes everything that mentions the node
- **WHEN** `v:alice` has a label, a name, `(v:alice v:worksAt v:acme)` = e1 with `(e1 v:confidence 0.8)`, and `(v:bob v:knows v:alice)`, and `MATCH (n {name:'Alice'}) DETACH DELETE n` runs
- **THEN** all five statements are retracted in one transaction, and `v:acme` and `v:bob` keep their other statements

#### Scenario: History still shows the node
- **WHEN** after that, `USE AS OF <t − 1> MATCH (n {name:'Alice'})-[r]->(c) RETURN c` runs
- **THEN** `v:acme` is returned

### Requirement: Schema and namespace rules apply to Cypher writes
Every statement written by a Cypher clause SHALL pass through the same schema checks as the API (`sys:valueType`, `sys:unique`, `sys:cardinality`) and the reserved-namespace rule. A violation MUST fail the query with the corresponding typed error (`ValueTypeMismatch`, `UniqueViolation`, `ReservedNamespace`, `CascadeLimitExceeded`), and the transaction MUST leave no trace.

#### Scenario: valueType mismatch
- **WHEN** `v:age` has `sys:valueType xsd:integer` and `CREATE (n {age: 'old'})` runs
- **THEN** the query fails with `ValueTypeMismatch`

#### Scenario: Writing a reserved predicate
- **WHEN** `MATCH (n {name:'Alice'}) SET n.`sys:reason` = 'x'` runs
- **THEN** the query fails with `ReservedNamespace`

#### Scenario: Cascade limit
- **WHEN** a detach delete would cascade more statements than the transaction's `max_cascade`
- **THEN** the query fails with `CascadeLimitExceeded`, and the store is unchanged

### Requirement: Writes and time clauses
Writes SHALL always apply to the current state. A query that contains a write clause and a query-level `USE` clause selecting anything other than transaction time `Now` with unfiltered valid time MUST fail with `Unsupported` before execution. Historical reads inside `CALL { USE … }` subqueries SHALL be allowed in a write query. Writes that target a statement bound from a historical scope SHALL follow the core operation rules: retracting a retracted statement is a no-op, and superseding it fails with `NotLive`.

#### Scenario: Query-level AS OF with a write rejected
- **WHEN** `USE AS OF 5 MATCH (n) SET n.x = 1` runs through the write entry point
- **THEN** it fails with `Unsupported`

#### Scenario: Restore a past value from a historical scope
- **WHEN** alice's `v:title` was "Dr" as of tx 5 and is "Prof" now, and `CALL { USE AS OF 5 MATCH (a {`@id`: 'v:alice'}) RETURN a.title AS old } MATCH (n {`@id`: 'v:alice'}) SET n.title = old` runs
- **THEN** alice's live title is "Dr"

### Requirement: Unsupported write features
`FOREACH`, `CALL { … } IN TRANSACTIONS`, dynamic labels or property keys in write clauses, and write clauses inside `EXISTS { }` MUST fail with `Unsupported` before execution.

#### Scenario: FOREACH in a write query
- **WHEN** `MATCH (n) FOREACH (x IN [1,2] | CREATE (:T {v: x}))` runs through the write entry point
- **THEN** it fails with `Unsupported` naming `FOREACH`, and nothing is written
