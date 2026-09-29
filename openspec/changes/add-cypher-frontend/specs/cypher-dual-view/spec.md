## Purpose

Defines the Cypher dual view: every statement eid is both a relationship and a node with the implicit label `:Statement`. Cypher can then read and write layers (statements about statements) while standard Cypher queries keep their Neo4j meaning.

## ADDED Requirements

### Requirement: Relationship variable in node position
A variable bound to a relationship SHALL be usable in a node position of a later pattern, in the same `MATCH` clause or in any later clause while it is in scope. In node position it SHALL denote the same statement eid, so a relationship whose subject or object is that eid matches it. Using the variable in node position SHALL NOT count as a relationship traversal for the isomorphism constraint.

#### Scenario: Belief supporting a relationship in the same MATCH
- **WHEN** e1 = `(v:alice v:WORKS_AT v:acme)`, `(v:belief9 rdf:type v:Belief)`, `(v:belief9 v:SUPPORTED_BY e1)` and `(e1 v:confidence 0.8)` are live, and `MATCH (a)-[r:WORKS_AT]->(c), (b:Belief)-[:SUPPORTED_BY]->(r) RETURN a, c, r.confidence, b` runs
- **THEN** one row is returned: `a = v:alice`, `c = v:acme`, `r.confidence = 0.8`, `b = v:belief9`

#### Scenario: Relationship variable reused in a later clause
- **WHEN** `MATCH (a)-[r:WORKS_AT]->(c) WITH r MATCH (b)-[:SUPPORTED_BY]->(r) RETURN b` runs on the same store
- **THEN** one row with `b = v:belief9` is returned

#### Scenario: Statement as subject of a relationship
- **WHEN** `(e1 v:assertedBy v:agent7)` is live and `MATCH ()-[r:WORKS_AT]->() MATCH (r)-[:assertedBy]->(who) RETURN who` runs
- **THEN** `who` is `v:agent7`

#### Scenario: Node-position use is not a traversal
- **WHEN** `MATCH (a)-[r:WORKS_AT]->(c), (x)-[s]->(r) RETURN count(*) AS n` runs on the store from the first scenario
- **THEN** `n` is 1, with `s` bound to the `SUPPORTED_BY` statement

### Requirement: Statements reached through relationships are nodes
When a relationship's object (or subject) is a statement, the node variable at that end SHALL bind to the statement in its node form, without the variable having been bound as a relationship first.

#### Scenario: Unbound node variable binds to a statement
- **WHEN** `MATCH (b:Belief)-[:SUPPORTED_BY]->(x) RETURN x, labels(x) AS l, x.confidence AS conf` runs on the store from the first scenario
- **THEN** `x` is a Node whose element id is that of e1, `l` contains `"Statement"`, and `conf` is 0.8

### Requirement: Implicit Statement label
Every statement in node form SHALL carry the implicit label `Statement`, in addition to the labels given by its own `rdf:type` statements. `MATCH (s:Statement)` SHALL enumerate every visible statement whose predicate is not in the `sys:` namespace, including property statements and label statements, each exactly once. `labels()` of a statement node SHALL list `"Statement"` first, followed by its other labels in ascending order.

#### Scenario: Enumerate statements
- **WHEN** the only live statements are `(v:alice v:name "Alice")` and `(v:alice v:knows v:bob)`, and `MATCH (s:Statement) RETURN count(s) AS n` runs
- **THEN** `n` is 2

#### Scenario: Typed statement
- **WHEN** `(e1 rdf:type v:Claim)` is live and `MATCH (s:Statement:Claim) RETURN labels(s) AS l` runs
- **THEN** `l` is `["Statement", "Claim"]`

#### Scenario: Statement nodes are not plain nodes
- **WHEN** `MATCH (n) RETURN count(n) AS c` runs on a store holding only `(v:alice v:knows v:bob)` with eid e1 and `(e1 v:confidence 0.8)`
- **THEN** `c` is 2 (`v:alice` and `v:bob`), not 3

### Requirement: startNode, endNode and type on either form
`startNode(x)`, `endNode(x)` and `type(x)` SHALL accept a relationship or a statement in node form. They SHALL return the statement's subject, the statement's object and the resolved name of its predicate. A subject or object that is itself a statement SHALL be returned as a statement node. The object of a property statement SHALL be returned as its literal value.

#### Scenario: Functions on the relationship form
- **WHEN** `MATCH (a)-[r:WORKS_AT]->(c) RETURN startNode(r) = a AS s, endNode(r) = c AS e, type(r) AS t` runs
- **THEN** the row is `s = true`, `e = true`, `t = "WORKS_AT"`

#### Scenario: Functions on the node form
- **WHEN** `MATCH (:Belief)-[:SUPPORTED_BY]->(x) RETURN elementId(startNode(x)) AS s, type(x) AS t` runs
- **THEN** `s` is `"urn:tiramemsu:v:alice"` and `t` is `"WORKS_AT"`

#### Scenario: Property statement end is a literal
- **WHEN** `MATCH (s:Statement) WHERE type(s) = 'name' RETURN endNode(s) AS v` runs on a store with `(v:alice v:name "Alice")`
- **THEN** `v` is the string `"Alice"`

### Requirement: Properties and relationships of statements
Literal-valued statements whose subject is a statement eid SHALL be properties of that statement, accessible as `r.key` on the relationship form and as `x.key` on the node form. Node- or statement-valued statements whose subject is a statement eid SHALL be relationships that start at the statement node. Nesting SHALL have no depth limit.

#### Scenario: Two-level layer
- **WHEN** e1 = `(v:alice v:WORKS_AT v:acme)`, e7 = `(v:belief9 v:SUPPORTED_BY e1)` and `(e7 v:method "llm-extraction")` are live, and `MATCH ()-[r:WORKS_AT]->(), ()-[s:SUPPORTED_BY]->(r) RETURN s.method AS m` runs
- **THEN** `m` is `"llm-extraction"`

### Requirement: Returned form of a dual-view variable
A variable SHALL be returned in the form in which it was first bound. A variable first bound in a relationship position SHALL be returned as a Relationship value, even if it is later used in node position. A variable first bound in node position to a statement SHALL be returned as a Node value with the `Statement` label. Both forms SHALL carry the same element id, and they SHALL compare equal under `=`.

#### Scenario: Same eid, two forms
- **WHEN** `MATCH ()-[r:WORKS_AT]->() MATCH (:Belief)-[:SUPPORTED_BY]->(x) RETURN r, x, r = x AS same, elementId(r) = elementId(x) AS sameId` runs
- **THEN** `r` is a Relationship, `x` is a Node labelled `Statement`, and `same` and `sameId` are both `true`

### Requirement: Standard Cypher queries keep their meaning
A query that uses relationship variables only in relationship positions, and node variables only in node positions, SHALL return the same results as openCypher semantics over the property-graph projection defined by `cypher-read`. Statements SHALL appear as nodes only when reached through the dual view or through the `:Statement` label. A variable first bound as a node MUST NOT be used in a relationship position, and doing so MUST fail with a `Parse` error.

#### Scenario: Unlabelled relationship count unaffected by layers
- **WHEN** e1 = `(v:alice v:knows v:bob)` and e2 = `(v:carol v:SUPPORTED_BY e1)` are live and `MATCH (a)-[r]->(b) WHERE NOT b:Statement RETURN count(r) AS c` runs
- **THEN** `c` is 1
- **AND** `MATCH (a)-[r]->(b) RETURN count(r)` returns 2, because the statement-valued relationship has a statement node as its end

#### Scenario: Node variable in relationship position rejected
- **WHEN** `MATCH (:Belief)-[:SUPPORTED_BY]->(x) MATCH ()-[x]->() RETURN x` is compiled
- **THEN** it fails with a `Parse` error spanning the second `x`

### Requirement: Writing layers through the dual view
Write clauses SHALL accept a relationship variable, or a statement bound in node form, in node positions. `CREATE (b)-[:T]->(r)` SHALL create a statement whose object is r's eid, and `SET r.key = v` SHALL write a property statement whose subject is the eid. `DELETE x` on a statement node SHALL retract that statement with cascade.

#### Scenario: Attach a belief to a relationship
- **WHEN** `` MATCH (a {`@id`: 'v:alice'})-[r:WORKS_AT]->() CREATE (b:Belief {text: 'from CV'})-[:SUPPORTED_BY]->(r) RETURN b `` runs through the write entry point
- **THEN** a new statement `(b, v:SUPPORTED_BY, e1)` is live, where e1 is the `WORKS_AT` eid

#### Scenario: Annotate a relationship
- **WHEN** `MATCH ()-[r:WORKS_AT]->() SET r.confidence = 0.95` runs where no confidence existed
- **THEN** `(e1 v:confidence 0.95)` is live

#### Scenario: Delete a statement through its node form
- **WHEN** `MATCH (:Belief)-[:SUPPORTED_BY]->(x) DELETE x` runs
- **THEN** e1 is retracted, and the `SUPPORTED_BY` statement pointing at it is retracted by cascade
