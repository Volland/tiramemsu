# path-lowering Specification

## Purpose
Defines how SPARQL property paths and Cypher variable-length, `shortestPath` and `allShortestPaths` patterns are evaluated once native path evaluation is available. This replaces the front ends' interim `Unsupported` behaviour for these constructs with results that follow each dialect's semantics.

## Requirements

### Requirement: SPARQL recursive property paths
A SPARQL property path that contains `*`, `+` or `?` SHALL be evaluated with `REACH` semantics. Each solution SHALL bind the path's subject and object variables, and a pair SHALL appear at most once per path pattern, whatever the number of witnessing paths or eids. The results SHALL follow SPARQL 1.1 property path semantics, including zero-length matches for `*` and `?`.

#### Scenario: One-or-more from a bound subject
- **WHEN** the store holds `(:a :knows :b)` and `(:b :knows :c)`, and `SELECT ?x WHERE { :a :knows+ ?x }` is run
- **THEN** the solutions are exactly `?x = :b` and `?x = :c`

#### Scenario: Zero-or-more includes the subject
- **WHEN** `SELECT ?x WHERE { :a :knows* ?x }` is run on the same store
- **THEN** the solutions are exactly `:a`, `:b` and `:c`

#### Scenario: Zero-or-more from a term not in the graph
- **WHEN** `SELECT ?x WHERE { :nobody :knows* ?x }` is run and `:nobody` appears in no statement
- **THEN** the only solution is `?x = :nobody`

#### Scenario: Zero-or-one
- **WHEN** `SELECT ?x WHERE { :a :knows? ?x }` is run on the same store
- **THEN** the solutions are exactly `:a` and `:b`

#### Scenario: No duplicate solutions over a diamond
- **WHEN** the store holds `(:a :p :b)`, `(:a :p :c)`, `(:b :p :d)` and `(:c :p :d)`, and `SELECT ?x WHERE { :a :p+ ?x }` is run
- **THEN** `?x = :d` appears in exactly one solution

#### Scenario: Object bound, subject variable
- **WHEN** `SELECT ?x WHERE { ?x :knows+ :c }` is run on the chain store
- **THEN** the solutions are exactly `:a` and `:b`

#### Scenario: Both endpoints constant
- **WHEN** `ASK { :a :knows+ :c }` and `ASK { :c :knows+ :a }` are run on the chain store
- **THEN** the first returns true and the second returns false

#### Scenario: Same variable at both ends
- **WHEN** the store holds `(:a :p :b)` and `(:b :p :a)`, and `SELECT ?x WHERE { VALUES ?x { :a } ?x :p+ ?x }` is run
- **THEN** the only solution is `?x = :a`

#### Scenario: Complex expression
- **WHEN** `SELECT ?x WHERE { :a (:knows/^:knows)+ ?x }` is run on a store holding only `(:a :knows :b)` and `(:e :knows :b)`
- **THEN** the solutions are exactly `:a` and `:e`

### Requirement: SPARQL non-recursive property paths
A SPARQL property path that contains no `*`, `+` or `?` (only `/`, `|`, `^` and IRIs) SHALL be evaluated by the SPARQL 1.1 translation to joins and unions of triple patterns. Its multiplicity SHALL follow that translation, and it SHALL NOT need a bound endpoint.

#### Scenario: Sequence with both ends unbound
- **WHEN** the store holds `(:a :knows :b)` and `(:b :worksAt :acme)`, and `SELECT ?x ?y WHERE { ?x :knows/:worksAt ?y }` is run
- **THEN** the only solution is `?x = :a, ?y = :acme`, and no bound-endpoint error is raised

#### Scenario: Alternation with both ends unbound
- **WHEN** `SELECT ?x ?y WHERE { ?x :knows|:likes ?y }` is run
- **THEN** it returns every `:knows` and `:likes` pair, with no error

#### Scenario: Inverse path
- **WHEN** `(v:alice v:worksAt v:acme)` is live and `SELECT ?p WHERE { v:acme ^v:worksAt ?p }` is run
- **THEN** one row with `p = v:alice` is returned, and no bound-endpoint error is raised for a query with both ends unbound

### Requirement: SPARQL endpoint binding
The subject or object of a recursive SPARQL property path SHALL count as bound when it is:
- a constant;
- a variable bound by `VALUES`;
- a variable bound by another pattern in the same group that can be evaluated before the path.

When neither endpoint is bound, the query SHALL fail with an `Unsupported` error that names the bound-endpoint requirement.

#### Scenario: Endpoint bound by a preceding pattern
- **WHEN** `SELECT ?p ?x WHERE { ?p a :Person . ?p :knows+ ?x }` is run
- **THEN** it returns, for every `:Person`, each node reachable over `:knows+`

#### Scenario: Both endpoints free
- **WHEN** `SELECT ?x ?y WHERE { ?x :knows+ ?y }` is run
- **THEN** the query fails with an `Unsupported` error that names the bound-endpoint requirement

### Requirement: SPARQL paths honour temporal scope
A SPARQL property path SHALL be evaluated under the view of its scope: the query-level `FROM <urn:tiramemsu:tm:…>` default, or the innermost enclosing `SERVICE <urn:tiramemsu:tm:…>` group, with the same meaning as for triple patterns. A `tm:` IRI inside `GRAPH` is not a time scope: it fails with a `Parse` error that names `SERVICE`.

#### Scenario: Path inside an asOf service group
- **WHEN** `(:a :knows :b)` was asserted in tx 10 and `(:b :knows :c)` in tx 20, and `SELECT ?x WHERE { SERVICE <urn:tiramemsu:tm:asOf/15> { :a :knows+ ?x } }` is run
- **THEN** the only solution is `?x = :b`

#### Scenario: Before and after in one query
- **WHEN** the same store is queried with `SELECT ?x WHERE { :a :knows+ ?x FILTER NOT EXISTS { SERVICE <urn:tiramemsu:tm:asOf/15> { :a :knows+ ?x } } }`
- **THEN** the only solution is `?x = :c`

#### Scenario: GRAPH is not a time scope for paths
- **WHEN** `SELECT ?x WHERE { GRAPH <urn:tiramemsu:tm:asOf/15> { :a :knows+ ?x } }` is run
- **THEN** the query fails with a `Parse` error that names `SERVICE`

#### Scenario: Valid-time default applies to paths
- **WHEN** a query with `FROM <urn:tiramemsu:tm:validAt/2023-06-01>` evaluates `:a :worksAt/:locatedIn* ?x`, and `(:a :worksAt :acme)` is valid only `[2020-01-01, 2022-01-01)`
- **THEN** no solution reaches `:acme`

### Requirement: SPARQL paths across layers
A SPARQL property path SHALL accept `sys:subject`, `sys:object` and `sys:predicate`, and their inverses, as virtual layer hops. Reifier variables (`~ ?r`) SHALL be usable as path endpoints.

#### Scenario: From a belief to the entities of its supporting fact
- **WHEN** the statement `(:alice :worksAt :acme)` has eid `e1`, `(:belief9 :supportedBy e1)` is live, and `SELECT ?x WHERE { :belief9 :supportedBy/(sys:subject|sys:object)+ ?x }` is run
- **THEN** the solutions are exactly `:alice` and `:acme`

### Requirement: Unsupported SPARQL path forms
A negated property set (`!p` or `!(p|^q)`) SHALL fail with an `Unsupported` error that names negated property sets.

#### Scenario: Negated property set
- **WHEN** `SELECT ?x WHERE { :a !:knows ?x }` is run
- **THEN** the query fails with an `Unsupported` error that names negated property sets

### Requirement: Cypher variable-length relationships
A Cypher relationship pattern with a `*` quantifier SHALL be evaluated with `TRAIL` semantics, returning one row per matching trail (bag semantics). The quantifier forms SHALL be:
- `*`: 1 to unbounded;
- `*n`: exactly n;
- `*m..n`;
- `*m..`: m to unbounded;
- `*..n`: 1 to n.

An unbounded upper limit SHALL be replaced by the database path hop cap (default 15), and reaching the cap SHALL NOT raise an error. The relationship types SHALL be:
- one type (`:T`);
- an alternation (`:T|U`);
- none, which matches every relationship the Cypher relationship view exposes.

The direction SHALL be:
- `->`: forward;
- `<-`: inverse;
- `-`: either direction per hop.

#### Scenario: Default bounds
- **WHEN** the store holds `(a KNOWS b)`, `(b KNOWS c)` and `(c KNOWS d)`, and `MATCH (x {id:'a'})-[:KNOWS*]->(y) RETURN y.id` is run
- **THEN** the rows are `b`, `c` and `d`

#### Scenario: Exact length
- **WHEN** `MATCH (x {id:'a'})-[:KNOWS*2]->(y) RETURN y.id` is run on the same store
- **THEN** the only row is `c`

#### Scenario: Range with zero minimum
- **WHEN** `MATCH (x {id:'a'})-[:KNOWS*0..1]->(y) RETURN y.id` is run
- **THEN** the rows are `a` and `b`

#### Scenario: Upper bound only
- **WHEN** `MATCH (x {id:'a'})-[:KNOWS*..2]->(y) RETURN y.id` is run
- **THEN** the rows are `b` and `c`

#### Scenario: Unbounded pattern stops at the cap
- **WHEN** the store holds a `NEXT` chain of 20 hops from `n0`, and `MATCH (s {id:'n0'})-[:NEXT*]->(e) RETURN count(*)` is run with the default cap
- **THEN** the count is 15 and no error is raised

#### Scenario: Type alternation
- **WHEN** the store holds `(a KNOWS b)` and `(b LIKES c)`, and `MATCH (x {id:'a'})-[:KNOWS|LIKES*]->(y) RETURN y.id` is run
- **THEN** the rows are `b` and `c`

#### Scenario: Any type
- **WHEN** the store holds `(a KNOWS b)`, `(b LIKES c)` and the property `(b name "Bob")`, and `MATCH (x {id:'a'})-[*]->(y) RETURN y` is run
- **THEN** the rows are the nodes `b` and `c`, and the literal `"Bob"` is never returned as a node

#### Scenario: Incoming direction
- **WHEN** `MATCH (y {id:'c'})<-[:KNOWS*]-(x) RETURN x.id` is run on the `KNOWS` chain
- **THEN** the rows are `b` and `a`

#### Scenario: Undirected
- **WHEN** the store holds `(a KNOWS b)` and `(c KNOWS b)`, and `MATCH (x {id:'a'})-[:KNOWS*2]-(y) RETURN y.id` is run
- **THEN** the only row is `c`

#### Scenario: Bag semantics over parallel relationships
- **WHEN** the store holds two `CALLED` relationships from `a` to `b` created with `CREATE`, and `MATCH (x {id:'a'})-[:CALLED*1..1]->(y) RETURN count(*)` is run
- **THEN** the count is 2

### Requirement: Cypher path and relationship-list bindings
A named path `p = (…)-[…*…]->(…)` SHALL bind a path value whose nodes and relationships appear in pattern order, from the leftmost node to the rightmost:
- `length(p)` SHALL equal the hop count;
- `nodes(p)` SHALL return hops + 1 nodes;
- `relationships(p)` SHALL return the relationships as the same relationship values (same eids) that a single-hop `MATCH` binds.

A relationship variable on a variable-length pattern (`[r:T*]`) SHALL bind the list of those relationships. A virtual layer hop in a path SHALL appear as a relationship whose `type()` is the CURIE `sys:subject`, `sys:object` or `sys:predicate`, and whose `startNode` and `endNode` are the two nodes it connects in the stored direction.

#### Scenario: Path value functions
- **WHEN** `(a KNOWS b)` has eid `e1` and `(b KNOWS c)` has eid `e2`, and `MATCH p = (x {id:'a'})-[:KNOWS*2]->(y) RETURN length(p), [n IN nodes(p) | n.id], [r IN relationships(p) | id(r)]` is run
- **THEN** the single row is `2`, `['a','b','c']`, `[e1, e2]`

#### Scenario: Path order with a bound right-hand node
- **WHEN** the store holds the `KNOWS` chain `a→b→c`, and `MATCH p = (x)-[:KNOWS*]->(y {id:'c'}) RETURN [n IN nodes(p) | n.id]` is run, so only `y` is bound before the path is evaluated
- **THEN** the rows are exactly `['b','c']` and `['a','b','c']`, each listed from left to right in pattern order

#### Scenario: Relationship list variable
- **WHEN** `(a KNOWS b)` has eid `e1` and `(b KNOWS c)` has eid `e2`, and `MATCH (x {id:'a'})-[rs:KNOWS*2]->(y) RETURN size(rs), [r IN rs | id(r)]` is run
- **THEN** the single row is `2`, `[e1, e2]`

#### Scenario: Virtual hop in a Cypher path
- **WHEN** ``MATCH p = (b:Belief)-[:SUPPORTED_BY|`sys:subject`*2]->(x) RETURN [r IN relationships(p) | type(r)]`` is run, and `b` supports statement `e1 = (alice WORKS_AT acme)`
- **THEN** the only row is `['SUPPORTED_BY', 'sys:subject']`, and `x` is `alice`

### Requirement: Cypher relationship isomorphism with paths
Within one `MATCH`, the relationships of a variable-length pattern SHALL be distinct from each other (trail). They SHALL also be distinct from the relationships bound by every other relationship pattern in the same `MATCH`, including other variable-length patterns. Rows that would reuse a relationship SHALL be dropped.

#### Scenario: Variable-length pattern avoids a fixed relationship
- **WHEN** the store holds `(a KNOWS b)` (eid `e1`) and `(b KNOWS c)` (eid `e2`), and `MATCH (x {id:'a'})-[r:KNOWS]->(y), (y)<-[:KNOWS*1..2]-(z) RETURN z.id` is run
- **THEN** no row is returned, because the only path from `b` against `KNOWS` reuses `e1`

#### Scenario: Two variable-length patterns share no relationship
- **WHEN** `MATCH (x {id:'a'})-[:KNOWS*]->(m), (m)<-[:KNOWS*]-(w) RETURN count(*)` is run on the `KNOWS` chain `a→b→c`
- **THEN** the count is 0, because every candidate row would walk back over an eid already used by the first pattern

### Requirement: Cypher shortest paths
`shortestPath(…)` SHALL be evaluated with `ANY_SHORTEST` semantics and `allShortestPaths(…)` with `ALL_SHORTEST` semantics, over a single variable-length relationship pattern. There is one result per distinct pair of bound endpoints: one path for `shortestPath`, all minimal paths for `allShortestPaths`. The minimum length SHALL be 0 or 1. Any other minimum SHALL fail with an `Unsupported` error. An unbounded upper limit SHALL be replaced by the database path hop cap.

#### Scenario: shortestPath between two bound nodes
- **WHEN** the store holds `(a R b)`, `(b R c)` and `(a R c)`, and `MATCH (x {id:'a'}), (y {id:'c'}), p = shortestPath((x)-[:R*]->(y)) RETURN length(p)` is run
- **THEN** the only row is `1`

#### Scenario: allShortestPaths returns every minimal path
- **WHEN** the store holds `(a R b)`, `(a R c)`, `(b R d)` and `(c R d)`, and `MATCH (x {id:'a'}), (y {id:'d'}), p = allShortestPaths((x)-[:R*]-(y)) RETURN [n IN nodes(p) | n.id]` is run
- **THEN** exactly the rows `['a','b','d']` and `['a','c','d']` are returned

#### Scenario: shortestPath is deterministic
- **WHEN** two shortest paths of equal length exist between `x` and `y`, and the same `shortestPath` query is run twice
- **THEN** both runs return the same path

#### Scenario: shortestPath with a larger minimum
- **WHEN** `MATCH p = shortestPath((x {id:'a'})-[:R*2..5]->(y)) RETURN p` is run
- **THEN** the query fails with an `Unsupported` error that names the shortest-path minimum length

#### Scenario: No path
- **WHEN** no `R` path exists from `x` to `y`, and `MATCH (x {id:'a'}), (y {id:'z'}), p = shortestPath((x)-[:R*]->(y)) RETURN p` is run
- **THEN** no row is returned; with `OPTIONAL MATCH`, `p` is NULL

### Requirement: Cypher endpoint binding
A Cypher variable-length or shortest-path pattern SHALL count as having a bound endpoint when either end node is fixed by a constant or parameter property, by an earlier clause, or by another pattern in the same `MATCH` that can be evaluated first. When neither end can be bound, the query SHALL fail with an `Unsupported` error that names the bound-endpoint requirement.

#### Scenario: Endpoint from a label scan
- **WHEN** `MATCH (x:Person)-[:KNOWS*1..2]->(y) RETURN x, y` is run
- **THEN** it returns the trails from every `:Person`, because the label pattern binds `x` first

#### Scenario: No bound endpoint
- **WHEN** `MATCH (x)-[:KNOWS*]->(y) RETURN x, y` is run
- **THEN** the query fails with an `Unsupported` error that names the bound-endpoint requirement

### Requirement: Cypher paths honour temporal scope
A Cypher variable-length or shortest-path pattern SHALL be evaluated under the view of its scope: the query's `USE AS OF`, `USE VALID AT` or `USE HISTORY` clause, or the enclosing `CALL { USE … }` subquery.

#### Scenario: Trail as of an earlier transaction
- **WHEN** `(a KNOWS b)` was asserted in tx 10 and `(b KNOWS c)` in tx 20, and `USE AS OF 15 MATCH (x {id:'a'})-[:KNOWS*]->(y) RETURN y.id` is run
- **THEN** the only row is `b`

#### Scenario: Shortest path in a time-scoped subquery
- **WHEN** a shortest path over `R` shrank from 3 hops to 1 hop in tx 50, and the query compares `CALL { USE AS OF 49 MATCH p = shortestPath((x {id:'a'})-[:R*]->(y {id:'d'})) RETURN length(p) AS before }` with the same pattern at now
- **THEN** `before` is 3 and the current length is 1

### Requirement: Unsupported Cypher path forms
The following SHALL fail with an `Unsupported` error that names the feature, and SHALL NOT be silently mis-evaluated:
- a variable-length relationship with a property map (e.g. `[:T*1..3 {since: 2020}]`);
- quantified path patterns (e.g. `((a)-[:T]->(b)){1,3}`);
- the path modes `WALK`, `SIMPLE`, `ACYCLIC` and `SHORTEST k`;
- `REPEATABLE ELEMENTS` combined with a variable-length pattern.

#### Scenario: Property map on a variable-length relationship
- **WHEN** `MATCH (x {id:'a'})-[:KNOWS*1..3 {since: 2020}]->(y) RETURN y` is run
- **THEN** the query fails with an `Unsupported` error that names property maps on variable-length relationships

#### Scenario: Quantified path pattern
- **WHEN** `MATCH (x {id:'a'}) ((m)-[:KNOWS]->(n)){1,3} (y) RETURN y` is run
- **THEN** the query fails with an `Unsupported` error that names quantified path patterns

#### Scenario: Repeatable elements with a variable-length pattern
- **WHEN** `MATCH REPEATABLE ELEMENTS (x {id:'a'})-[:KNOWS*1..3]->(y) RETURN y` is run
- **THEN** the query fails with an `Unsupported` error that names walk semantics

### Requirement: Dialect agreement on paths
For equivalent path queries within the hop cap, SPARQL reachability results and the distinct end nodes of Cypher trail results SHALL be identical over the same data and view.

#### Scenario: Differential reachability
- **WHEN** a fixture graph with cycles and parallel edges, of diameter below 15, is queried with `SELECT DISTINCT ?y WHERE { :a :KNOWS+ ?y }` and `MATCH (x {id:'a'})-[:KNOWS*]->(y) RETURN DISTINCT y`
- **THEN** both return the same set of nodes
