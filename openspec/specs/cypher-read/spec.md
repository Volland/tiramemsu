# cypher-read Specification

## Purpose
Defines how Tiramemsu evaluates read-only Cypher queries over the statement store: which openCypher constructs are accepted, what they mean on addressable triples, how results are encoded, and how unsupported or invalid input is reported.

## Requirements

### Requirement: Read-only query entry point
The system SHALL run Cypher read queries through a view handle with the signature `cypher(query, params)`, and it SHALL evaluate every pattern under that view's time selection unless the query overrides it. The result SHALL be a table: ordered column names and rows of Cypher values. A query run on a read-only view that contains a write clause (`CREATE`, `MERGE`, `SET`, `REMOVE`, `DELETE`, `DETACH DELETE`) MUST fail with an `Unsupported` error that names the write clause, before anything is executed.

#### Scenario: Read query on the current view
- **WHEN** the store holds `(v:alice rdf:type v:Person)` and `(v:alice v:name "Alice")` and `MATCH (p:Person) RETURN p.name AS name` runs on the now view
- **THEN** the result has exactly one column `name` and one row `["Alice"]`

#### Scenario: Write clause on a read-only view is rejected
- **WHEN** `CREATE (n:Person {name: 'Bob'})` runs through the view entry point
- **THEN** the query fails with `Unsupported` naming `CREATE`
- **AND** no transaction is recorded and the store is unchanged

#### Scenario: View time selection is the default
- **WHEN** `(v:alice v:name "Alice")` was asserted in tx 1 and retracted in tx 2, and `MATCH (n) WHERE n.name = 'Alice' RETURN n` runs on the view as of tx 1
- **THEN** one row is returned
- **AND** the same query on the now view returns zero rows

### Requirement: Statement classification into properties and relationships
The system SHALL present each visible statement to Cypher either as a property of its subject or as a relationship. A statement whose object is a literal SHALL be a property, and a statement whose object is an IRI, anonymous node, blank node or statement SHALL be a relationship. A predicate flagged `sys:isEdge true` SHALL always be presented as a relationship, and one flagged `sys:isEdge false` SHALL always be presented as a property. The flag SHALL be read as visible in the view of the pattern that is being matched. Statements with predicate `rdf:type` SHALL be presented only as labels, never as relationships or properties.

#### Scenario: Literal object is a property
- **WHEN** the store holds `(v:alice v:age 42)` and `MATCH (a)-[r]->(x) RETURN count(r) AS c` runs
- **THEN** `c` is 0
- **AND** `MATCH (a) RETURN a.age` returns 42 for `v:alice`

#### Scenario: Node object is a relationship
- **WHEN** the store holds `(v:alice v:knows v:bob)` and `MATCH (a)-[r:knows]->(b) RETURN b` runs
- **THEN** one row is returned, and `b` is the node `v:bob`
- **AND** `MATCH (a) WHERE a.knows IS NOT NULL RETURN a` returns zero rows

#### Scenario: sys:isEdge true forces the relationship view on a literal
- **WHEN** `(v:tag sys:isEdge true)` is live, the store holds `(v:alice v:tag "urgent")`, and `MATCH (a)-[r:tag]->(t) RETURN t, a.tag` runs
- **THEN** one row is returned, with `t` bound to the string `"urgent"` and `a.tag` null

#### Scenario: sys:isEdge false forces the property view on a node object
- **WHEN** `(v:homepage sys:isEdge false)` is live, the store holds `(v:alice v:homepage <https://alice.example/>)`, and `MATCH (a) RETURN a.homepage` runs for `v:alice`
- **THEN** the value is the string `"https://alice.example/"`
- **AND** `MATCH ()-[r:homepage]->() RETURN r` returns zero rows

#### Scenario: rdf:type is never a relationship
- **WHEN** the store holds `(v:alice rdf:type v:Person)` only and `MATCH ()-[r]->() RETURN r` runs
- **THEN** zero rows are returned

### Requirement: Node patterns and labels
A node pattern SHALL match nodes: IRIs, anonymous nodes and blank nodes that are the subject of a visible statement, or the object of a visible relationship. Statements and transactions SHALL NOT be matched by an unlabelled, unconstrained node pattern unless the dual view binds them (see `cypher-dual-view`). A label `:L` SHALL require a visible `(n rdf:type <resolved L>)` statement. Several labels (`:A:B`) SHALL all be required, and `:A|B` SHALL require at least one. Inline property maps SHALL require a visible property equal to each given value. Label and property-map constraints SHALL be existence tests, so duplicate or episodic statements never multiply result rows.

#### Scenario: Label matches rdf:type
- **WHEN** the store holds `(v:alice rdf:type v:Person)`, `(v:acme rdf:type v:Company)` and `(v:alice v:worksAt v:acme)`, and `MATCH (n:Person) RETURN n` runs
- **THEN** exactly one row with node `v:alice` is returned

#### Scenario: Multiple labels are conjunctive
- **WHEN** `v:alice` has labels `Person` and `Employee`, `v:bob` has only `Person`, and `MATCH (n:Person:Employee) RETURN n` runs
- **THEN** only `v:alice` is returned

#### Scenario: Label disjunction
- **WHEN** `MATCH (n:Person|Company) RETURN count(n) AS c` runs on the store from the first scenario
- **THEN** `c` is 2

#### Scenario: Duplicate label statements do not duplicate rows
- **WHEN** `(v:alice rdf:type v:Person)` exists as two live eids with disjoint valid intervals and `MATCH (n:Person) RETURN n` runs
- **THEN** exactly one row is returned

#### Scenario: Inline property map filters
- **WHEN** `MATCH (n:Person {name: 'Alice'}) RETURN n` runs and two people exist, named Alice and Bob
- **THEN** only the node named Alice is returned

#### Scenario: Numeric property map compares by value
- **WHEN** the store holds `(v:x v:score 30)` as an integer and `MATCH (n {score: 30.0}) RETURN n` runs
- **THEN** `v:x` is returned

#### Scenario: Unlabelled node scan excludes statements, transactions and class IRIs
- **WHEN** the store holds `(v:alice rdf:type v:Person)`, `(v:alice v:worksAt v:acme)` with eid e1, `(e1 v:confidence 0.8)` and tx metadata `(tx1 sys:author v:agent7)`, and `MATCH (n) RETURN n` runs
- **THEN** exactly the nodes `v:alice` and `v:acme` are returned

### Requirement: Relationship patterns
A relationship pattern SHALL match one visible relationship statement per row. For `(a)-[r:T]->(b)`, `a` SHALL be the subject, `T` the predicate and `b` the object, and `<-` SHALL reverse the roles. An undirected pattern `(a)-[r]-(b)` SHALL match each statement once in each orientation. A type disjunction `[:A|B]` SHALL match either predicate. An untyped pattern SHALL match every relationship except those with `sys:` predicates. Relationships are a bag of eids: two live statements with the same subject, predicate and object SHALL produce two rows. Inline relationship property maps SHALL filter on properties whose subject is the relationship's eid.

#### Scenario: Directed match
- **WHEN** the store holds `(v:alice v:worksAt v:acme)` and `MATCH (c)<-[:worksAt]-(p) RETURN p, c` runs
- **THEN** one row with `p = v:alice` and `c = v:acme` is returned

#### Scenario: Undirected match returns both orientations
- **WHEN** the store holds only `(v:alice v:knows v:bob)` and `MATCH (a)-[:knows]-(b) RETURN a, b` runs
- **THEN** two rows are returned: `(v:alice, v:bob)` and `(v:bob, v:alice)`

#### Scenario: Parallel edges are distinct rows
- **WHEN** two live statements `(v:alice v:called v:bob)` exist with eids e1 and e2 and `MATCH (:Person)-[r:called]->(b) RETURN count(r) AS c` runs with `v:alice` labelled `Person`
- **THEN** `c` is 2

#### Scenario: Type disjunction
- **WHEN** the store holds `(v:alice v:knows v:bob)` and `(v:alice v:worksAt v:acme)` and `MATCH (v)-[r:knows|worksAt]->(x) RETURN count(*) AS c` runs
- **THEN** `c` is 2

#### Scenario: Untyped pattern hides sys predicates
- **WHEN** statement e10 superseded e1, so `(e10 sys:supersedes e1)` is live, and `MATCH ()-[r]->() RETURN type(r)` runs
- **THEN** no row has type `sys:supersedes`

#### Scenario: Relationship property map
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` with `(e1 v:role "cto")`, e2 = `(v:bob v:worksAt v:acme)` with `(e2 v:role "dev")`, and `MATCH (p)-[r:worksAt {role: 'cto'}]->(c) RETURN p` runs
- **THEN** only `v:alice` is returned

### Requirement: Property access
The expression `x.key` SHALL evaluate over the visible property statements `(x, <resolved key>, literal)`. With none it SHALL be `null`, with exactly one distinct value it SHALL be that value, and with two or more distinct values it SHALL be a list of the distinct values, ordered by the lowest eid that holds each value. `keys(x)` SHALL return the resolved names of the property keys that have at least one visible value, and `properties(x)` SHALL return a map from each such key to its `x.key` value. Accessing a property of `null` SHALL yield `null`.

#### Scenario: Missing property is null
- **WHEN** `v:alice` has no `v:email` statement and `MATCH (n) WHERE n = $alice RETURN n.email AS e` runs
- **THEN** `e` is `null`

#### Scenario: Multi-valued property becomes a list
- **WHEN** `(v:alice v:nick "al")` is asserted in tx 1 and `(v:alice v:nick "ally")` in tx 2, and `RETURN` of `n.nick` runs for `v:alice`
- **THEN** the value is the list `["al", "ally"]`

#### Scenario: keys and properties
- **WHEN** `v:alice` has `v:name "Alice"` and `v:age 42`, and `RETURN keys(n), properties(n)` runs for `v:alice`
- **THEN** `keys(n)` contains exactly `"age"` and `"name"`
- **AND** `properties(n)` is `{age: 42, name: "Alice"}`

### Requirement: Relationship isomorphism by default
Within one `MATCH` or `OPTIONAL MATCH` clause, including all its comma-separated patterns, no two relationship pattern positions SHALL bind the same relationship eid. The constraint SHALL NOT apply across separate clauses. The prefix `MATCH DIFFERENT RELATIONSHIPS` SHALL be accepted and mean the same as the default.

#### Scenario: Same relationship not reused within a clause
- **WHEN** the store holds only `(v:a v:knows v:b)` and `MATCH (x)-[r1:knows]-(y)-[r2:knows]-(z) RETURN count(*) AS c` runs
- **THEN** `c` is 0

#### Scenario: Comma-separated patterns share the constraint
- **WHEN** the store holds only `(v:a v:knows v:b)` and `MATCH (x)-[r1:knows]->(y), (p)-[r2:knows]->(q) RETURN count(*) AS c` runs
- **THEN** `c` is 0

#### Scenario: Separate clauses may reuse a relationship
- **WHEN** the store holds only `(v:a v:knows v:b)` and `MATCH (x)-[r1:knows]->(y) MATCH (p)-[r2:knows]->(q) RETURN count(*) AS c` runs
- **THEN** `c` is 1

#### Scenario: Explicit default mode
- **WHEN** `MATCH DIFFERENT RELATIONSHIPS (x)-[r1:knows]-(y)-[r2:knows]-(z) RETURN count(*) AS c` runs on the same store
- **THEN** `c` is 0

### Requirement: REPEATABLE ELEMENTS opt-out
`MATCH REPEATABLE ELEMENTS` SHALL evaluate the clause under homomorphism, so relationship patterns within it may bind the same eid.

#### Scenario: Repeated relationship allowed
- **WHEN** the store holds only `(v:a v:knows v:b)` and `MATCH REPEATABLE ELEMENTS (x)-[r1:knows]-(y)-[r2:knows]-(z) RETURN count(*) AS c` runs
- **THEN** `c` is 2, the walks a→b→a and b→a→b

### Requirement: WHERE with three-valued logic
`WHERE` SHALL keep only rows whose predicate evaluates to `true`, and it SHALL drop rows that evaluate to `false` or `null`. Comparisons involving `null` SHALL yield `null`. `AND`, `OR`, `NOT` and `XOR` SHALL follow Cypher's three-valued truth tables. `IS NULL` and `IS NOT NULL` SHALL never yield `null`. `x IN list` SHALL yield `true` if any element equals `x`, `null` if there is no match but the list or `x` contains `null`, and `false` otherwise. Values of incomparable types SHALL compare as `null` under `<`, `<=`, `>` and `>=`, and as `false` under `=`.

#### Scenario: Null comparison drops the row
- **WHEN** `v:alice` has no `v:age` and `MATCH (n:Person) WHERE n.age > 30 RETURN n` runs
- **THEN** `v:alice` is not returned
- **AND** `MATCH (n:Person) WHERE NOT (n.age > 30) RETURN n` also does not return `v:alice`

#### Scenario: Three-valued connectives
- **WHEN** `RETURN null OR true AS a, null AND false AS b, null AND true AS c, NOT null AS d, null XOR true AS e` runs
- **THEN** the row is `a = true`, `b = false`, `c = null`, `d = null`, `e = null`

#### Scenario: IS NULL
- **WHEN** `MATCH (n:Person) WHERE n.age IS NULL RETURN n` runs
- **THEN** every `Person` without an `age` property is returned

#### Scenario: IN with null
- **WHEN** `RETURN 2 IN [1, null] AS a, 1 IN [1, null] AS b, 3 IN [1, 2] AS c` runs
- **THEN** the row is `a = null`, `b = true`, `c = false`

#### Scenario: String predicates
- **WHEN** people named "Alice", "Alfred" and "Bob" exist and `MATCH (n:Person) WHERE n.name STARTS WITH 'Al' AND n.name CONTAINS 'i' RETURN n.name` runs
- **THEN** only `"Alice"` is returned

### Requirement: OPTIONAL MATCH
`OPTIONAL MATCH` SHALL behave as a left outer join with the preceding rows. When the pattern has no match for a row, every variable the clause introduces SHALL be `null` and the row SHALL be kept. A `WHERE` attached to `OPTIONAL MATCH` SHALL be part of the join condition, not a filter on the result.

#### Scenario: Missing optional part yields nulls
- **WHEN** `v:bob` is a `Person` with no `worksAt` relationship and `MATCH (p:Person) OPTIONAL MATCH (p)-[r:worksAt]->(c) RETURN p, r, c` runs
- **THEN** the row for `v:bob` has `r = null` and `c = null`

#### Scenario: WHERE inside OPTIONAL MATCH keeps the outer row
- **WHEN** `v:alice` works at `v:acme` (named "Acme") and `MATCH (p {name:'Alice'}) OPTIONAL MATCH (p)-[:worksAt]->(c) WHERE c.name = 'Globex' RETURN p, c` runs
- **THEN** one row is returned with `c = null`

### Requirement: WITH projection and variable scope
`WITH` SHALL project rows exactly as `RETURN` does (aliases, `DISTINCT`, aggregation, `ORDER BY`/`SKIP`/`LIMIT`). Only the variables it projects SHALL remain in scope afterwards. A `WHERE` directly after `WITH` SHALL filter the projected rows. A later reference to a variable that is out of scope MUST fail at compile time with a `Parse` error whose span covers the reference.

#### Scenario: Filter on an aggregate through WITH
- **WHEN** `v:acme` has three employees, `v:globex` has one, and `MATCH (p)-[:worksAt]->(c) WITH c, count(p) AS n WHERE n > 1 RETURN c.name, n` runs
- **THEN** one row `("Acme", 3)` is returned

#### Scenario: Variable dropped by WITH is out of scope
- **WHEN** `MATCH (p)-[:worksAt]->(c) WITH c RETURN p` is compiled
- **THEN** it fails with a `Parse` error whose span covers the `p` in `RETURN p`

#### Scenario: WITH DISTINCT
- **WHEN** two people work at `v:acme` and `MATCH (p)-[:worksAt]->(c) WITH DISTINCT c RETURN count(c) AS n` runs
- **THEN** `n` is 1

### Requirement: RETURN projection
`RETURN` SHALL produce one column per item, in order. Each column SHALL be named by its alias, or otherwise by the item's expression text exactly as written in the query. `RETURN *` SHALL return every variable in scope, sorted by name. `RETURN DISTINCT` SHALL remove duplicate rows using Cypher equality, under which two `null`s are equivalent.

#### Scenario: Column naming
- **WHEN** `MATCH (n:Person) RETURN n.name, n.age AS years` runs
- **THEN** the columns are `n.name` and `years`

#### Scenario: RETURN DISTINCT treats nulls as equal
- **WHEN** two people have no `age` and `MATCH (n:Person) RETURN DISTINCT n.age AS a` runs
- **THEN** exactly one row has `a = null`

#### Scenario: RETURN star
- **WHEN** `MATCH (b)<-[r:worksAt]-(a) RETURN *` runs
- **THEN** the columns are `a`, `b`, `r`, in that order

### Requirement: ORDER BY, SKIP and LIMIT
`ORDER BY` SHALL sort by Cypher's value ordering, not by storage identifiers. Numbers of all types SHALL compare numerically, strings by Unicode code point, dates chronologically, and datetimes by their instant, whatever their timezone offsets. Across types the ascending order SHALL be map, node, relationship, list, path, datetime, date, string, boolean, number, with `null` last when ascending and first when descending. `SKIP` and `LIMIT` SHALL accept non-negative integer literals or parameters, and a negative or non-integer value MUST fail the query.

#### Scenario: Numeric ordering across integer and float
- **WHEN** scores 10, 9.5 and 100 are stored as integer, double and integer and `MATCH (n) WHERE n.score IS NOT NULL RETURN n.score ORDER BY n.score` runs
- **THEN** the order is 9.5, 10, 100

#### Scenario: Nulls last ascending, first descending
- **WHEN** ages 30, null and 20 exist and `MATCH (n:Person) RETURN n.age ORDER BY n.age DESC` runs
- **THEN** the order is null, 30, 20

#### Scenario: SKIP and LIMIT with parameters
- **WHEN** ten people exist and `MATCH (n:Person) RETURN n.name ORDER BY n.name SKIP $s LIMIT $l` runs with `s = 2` and `l = 3`
- **THEN** the 3rd, 4th and 5th names in sort order are returned

#### Scenario: Negative LIMIT fails
- **WHEN** `MATCH (n) RETURN n LIMIT -1` is compiled
- **THEN** it fails with a `Parse` error whose span covers `-1`

### Requirement: UNWIND
`UNWIND expr AS x` SHALL produce one row per element of a list value. It SHALL produce no rows for an empty list or `null`, and one row with `x` bound to the value when the value is not a list.

#### Scenario: Unwind a list
- **WHEN** `UNWIND [1, 2, 3] AS x RETURN x * 10 AS y` runs
- **THEN** the rows are 10, 20, 30

#### Scenario: Unwind null and empty list
- **WHEN** `UNWIND null AS x RETURN x` and `UNWIND [] AS x RETURN x` run
- **THEN** both return zero rows

#### Scenario: Unwind a parameter list into a match
- **WHEN** `UNWIND $names AS nm MATCH (p:Person {name: nm}) RETURN p.name` runs with `names = ["Alice", "Zed"]` and only Alice exists
- **THEN** one row `"Alice"` is returned

### Requirement: Aggregation
The system SHALL support `count(*)`, `count(expr)`, `sum`, `avg`, `min`, `max` and `collect`, each optionally with `DISTINCT`. The non-aggregated items of the same projection SHALL be the implicit grouping keys. Aggregates other than `count(*)` SHALL ignore `null`. With no grouping keys and no input rows, the projection SHALL return one row, in which `count` is 0, `collect` is `[]`, and the other aggregates are `null`. An aggregate inside `WHERE`, or nested in another aggregate, MUST fail with a `Parse` error.

#### Scenario: Implicit grouping
- **WHEN** `MATCH (p)-[:worksAt]->(c) RETURN c.name AS company, count(p) AS staff, collect(p.name) AS names` runs
- **THEN** there is one row per company, with `staff` equal to the size of `names`

#### Scenario: Aggregates on empty input
- **WHEN** `MATCH (n:Unicorn) RETURN count(n) AS c, collect(n) AS l, sum(n.x) AS s` runs and no `Unicorn` exists
- **THEN** exactly one row `c = 0`, `l = []`, `s = null` is returned

#### Scenario: count ignores null and DISTINCT removes duplicates
- **WHEN** ages 30, 30 and null exist and `MATCH (n:Person) RETURN count(n.age) AS a, count(DISTINCT n.age) AS b, count(*) AS c` runs
- **THEN** the row is `a = 2`, `b = 1`, `c = 3`

#### Scenario: Aggregate in WHERE is rejected
- **WHEN** `MATCH (n) WHERE count(n) > 1 RETURN n` is compiled
- **THEN** it fails with a `Parse` error whose span covers `count(n)`

### Requirement: CALL subqueries
`CALL { … }` SHALL be supported in two forms. An uncorrelated subquery SHALL be evaluated once and combined with every incoming row as a cross product. A subquery that starts with an importing `WITH v1, v2` SHALL be evaluated once per incoming row with the imported variables bound. Each subquery result row SHALL be appended to its outer row, and an outer row with no subquery rows SHALL be dropped. A subquery that aggregates without grouping keys SHALL therefore always return exactly one row per outer row. Variables returned by the subquery MUST NOT shadow outer variables, and doing so MUST fail with a `Parse` error.

#### Scenario: Uncorrelated subquery is a cross product
- **WHEN** 2 `Person` and 3 `Company` nodes exist and `MATCH (p:Person) CALL { MATCH (c:Company) RETURN c } RETURN count(*) AS n` runs
- **THEN** `n` is 6

#### Scenario: Importing WITH with per-row aggregation
- **WHEN** `v:acme` has 3 employees and `v:initech` has none, and `MATCH (c:Company) CALL { WITH c OPTIONAL MATCH (p)-[:worksAt]->(c) RETURN count(p) AS staff } RETURN c.name, staff` runs
- **THEN** it returns `("Acme", 3)` and `("Initech", 0)`

#### Scenario: Empty correlated subquery drops the outer row
- **WHEN** `MATCH (c:Company) CALL { WITH c MATCH (p)-[:worksAt]->(c) RETURN p } RETURN c.name, p.name` runs on the same store
- **THEN** no row has `c.name = "Initech"`

#### Scenario: Shadowing an outer variable is rejected
- **WHEN** `MATCH (p:Person) CALL { MATCH (p:Company) RETURN p } RETURN p` is compiled
- **THEN** it fails with a `Parse` error whose span covers the subquery's `RETURN p`

### Requirement: UNION
`UNION` SHALL combine the results of queries that have identical column names, removing duplicate rows, and `UNION ALL` SHALL keep duplicates. Differing column names MUST fail with a `Parse` error.

#### Scenario: UNION removes duplicates
- **WHEN** `MATCH (n:Person) RETURN n.name AS x UNION MATCH (n:Person) RETURN n.name AS x` runs with two people
- **THEN** two rows are returned
- **AND** the same query with `UNION ALL` returns four rows

#### Scenario: Column mismatch
- **WHEN** `RETURN 1 AS a UNION RETURN 2 AS b` is compiled
- **THEN** it fails with a `Parse` error

### Requirement: Existential subqueries and pattern predicates
`EXISTS { MATCH … [WHERE …] }`, `EXISTS { pattern }`, and a bare pattern used as a predicate in `WHERE` SHALL evaluate to `true` if at least one match exists for the current row, and to `false` otherwise. They SHALL never yield `null` and never multiply rows.

#### Scenario: NOT EXISTS
- **WHEN** `v:bob` has no employer and `MATCH (p:Person) WHERE NOT EXISTS { (p)-[:worksAt]->() } RETURN p` runs
- **THEN** `v:bob` is returned and employed people are not

#### Scenario: Pattern predicate does not multiply rows
- **WHEN** `v:alice` has two `worksAt` relationships and `MATCH (p:Person) WHERE (p)-[:worksAt]->() RETURN p` runs
- **THEN** `v:alice` appears exactly once

### Requirement: Expressions and built-in functions
The system SHALL evaluate literals (integer, float, string, boolean, `null`, list, map), parameters, arithmetic, string concatenation with `+`, comparison, `=~` regular expressions, list indexing and slicing, map projection (`n {.name, k: expr}`), simple and searched `CASE`, and list comprehension over list values (`[x IN list WHERE p | e]`). It SHALL provide these functions with openCypher semantics: `id`, `elementId`, `labels`, `type`, `keys`, `properties`, `startNode`, `endNode`, `coalesce`, `size`, `head`, `last`, `range`, `toString`, `toInteger`, `toFloat`, `toBoolean`, `toLower`, `toUpper`, `trim`, `ltrim`, `rtrim`, `substring`, `replace`, `split`, `left`, `right`, `reverse`, `abs`, `ceil`, `floor`, `round`, `sign`, `sqrt`, `date`, `datetime`, `localdatetime`, `timestamp`, `nodes`, `relationships` and `length`. Calling any other function, or using a pattern comprehension, MUST fail with `Unsupported` naming it.

#### Scenario: CASE and coalesce
- **WHEN** `MATCH (n:Person) RETURN n.name, CASE WHEN n.age >= 18 THEN 'adult' ELSE 'minor' END AS k, coalesce(n.nick, n.name) AS shown` runs
- **THEN** every row has `k` in {"adult", "minor"}, and `shown` is the nickname if present, else the name

#### Scenario: List comprehension
- **WHEN** `RETURN [x IN range(1, 5) WHERE x % 2 = 1 | x * x] AS l` runs
- **THEN** `l` is `[1, 9, 25]`

#### Scenario: Unknown function
- **WHEN** `RETURN apoc.text.clean('x')` is compiled
- **THEN** it fails with `Unsupported` naming `apoc.text.clean`

#### Scenario: Pattern comprehension unsupported
- **WHEN** `MATCH (a) RETURN [(a)-->(b) | b.name]` is compiled
- **THEN** it fails with `Unsupported` naming pattern comprehension

### Requirement: Query parameters
`$name` SHALL be bound from the parameter map supplied with the query, and it MAY appear anywhere openCypher permits an expression, including inline property-map values, `SKIP`, `LIMIT` and `UNWIND`. A parameter that is referenced but not supplied MUST fail compilation with a `Parse` error that names the parameter and spans its reference. Supplied parameters that are not referenced SHALL be ignored.

#### Scenario: Parameter in a property map
- **WHEN** `MATCH (n:Person {name: $who}) RETURN n` runs with `who = "Alice"`
- **THEN** the Alice node is returned

#### Scenario: Missing parameter
- **WHEN** `MATCH (n {name: $who}) RETURN n` runs with an empty parameter map
- **THEN** it fails with a `Parse` error naming `who`, and nothing is executed

### Requirement: Result value model
Stored values SHALL be returned as Cypher values: integers as Integer; booleans as Boolean; doubles and decimals as Float; plain and language-tagged strings as String (the language tag is dropped); dates as Date; datetimes that carry a timezone as DateTime with their stored offset, and datetimes without a timezone as LocalDateTime, both with millisecond precision (offsets are never normalised to UTC); and other datatypes as a String holding the lexical form. A node SHALL be returned as a Node value carrying its element id, its labels and its properties. A relationship SHALL be returned as a Relationship value carrying its element id, type, start and end element ids and properties. Lists, maps and paths SHALL be returned structurally. Every value SHALL have a JSON encoding.

#### Scenario: Scalar round trip of types
- **WHEN** the store holds `v:x` with `v:i 7`, `v:f 1.5`, `v:b true`, `v:d "2025-03-01"^^xsd:date`, `v:t "2025-03-01T10:00:00+02:00"^^xsd:dateTime` and `v:s "hé"@fr`, and `MATCH (n) WHERE n = $x RETURN n.i, n.f, n.b, n.d, n.t, n.s` runs
- **THEN** the values are Integer 7, Float 1.5, Boolean true, Date 2025-03-01, DateTime 2025-03-01T10:00:00.000+02:00 and String "hé"

#### Scenario: DateTime equality compares instants
- **WHEN** `v:x` has `v:a "2026-03-01T12:00:00+02:00"^^xsd:dateTime` and `v:b "2026-03-01T10:00:00Z"^^xsd:dateTime`, and `MATCH (n) WHERE n = $x RETURN n.a = n.b AS eq, n.a AS a, n.b AS b` runs
- **THEN** `eq` is true, `a` is DateTime 2026-03-01T12:00:00.000+02:00 and `b` is DateTime 2026-03-01T10:00:00.000Z
- **AND** `MATCH (n {a: datetime('2026-03-01T10:00:00Z')}) RETURN n` returns `v:x`

#### Scenario: Date-time without timezone reads as LocalDateTime
- **WHEN** `v:x` has `v:l "2026-03-01T09:00:00"^^xsd:dateTime` and `MATCH (n) WHERE n = $x RETURN n.l AS l` runs
- **THEN** `l` is LocalDateTime 2026-03-01T09:00:00.000

#### Scenario: Node and relationship values
- **WHEN** `MATCH (a:Person)-[r:worksAt]->(c) RETURN a, r` runs for `(v:alice v:worksAt v:acme)` with eid e1
- **THEN** `a` is a Node with element id `urn:tiramemsu:v:alice`, labels `["Person"]` and its properties
- **AND** `r` is a Relationship with type `worksAt`, start element id `urn:tiramemsu:v:alice`, end element id `urn:tiramemsu:v:acme`, and the element id of e1

### Requirement: Fixed-length named paths
A named path `p = (a)-[…]->(b)…` over a fixed-length pattern SHALL bind a Path value made of the matched nodes and relationships in pattern order. `nodes(p)`, `relationships(p)` and `length(p)` SHALL return its nodes, its relationships and its relationship count.

#### Scenario: Two-hop named path
- **WHEN** `(v:a v:knows v:b)` and `(v:b v:knows v:c)` exist and `MATCH p = (x)-[:knows]->()-[:knows]->(z) RETURN length(p) AS l, [n IN nodes(p) | elementId(n)] AS ids` runs
- **THEN** `l` is 2 and `ids` is `["urn:tiramemsu:v:a", "urn:tiramemsu:v:b", "urn:tiramemsu:v:c"]`

### Requirement: Volatile virtual properties
When the view's transaction-time selector is `Now`, `n.key` SHALL also resolve against the volatile table entry for `(n, <resolved key>)`. A visible property statement with the same key SHALL take precedence over the volatile value. Under `AS OF` or `HISTORY` views, volatile values SHALL be absent (`null`). Volatile keys SHALL appear in `keys(n)` and `properties(n)` only under the `Now` selector. Cypher write clauses SHALL never write to the volatile table.

#### Scenario: Volatile value visible now
- **WHEN** volatile `(v:alice, v:lastSeen) = datetime 2026-09-29T10:00Z` is set, there is no `v:lastSeen` statement, and `MATCH (n) WHERE n = $alice RETURN n.lastSeen` runs on the now view
- **THEN** the value is DateTime 2026-09-29T10:00:00.000Z

#### Scenario: Triple wins on collision
- **WHEN** both the volatile entry `(v:alice, v:lastSeen)` and the statement `(v:alice v:lastSeen "2020-01-01T00:00:00Z"^^xsd:dateTime)` exist
- **THEN** `n.lastSeen` returns 2020-01-01T00:00:00.000Z

#### Scenario: Volatile absent in the past
- **WHEN** the same query runs with `USE AS OF 5`, or on a history view
- **THEN** `n.lastSeen` is `null`

#### Scenario: SET writes a triple, not volatile
- **WHEN** `MATCH (n) WHERE n = $alice SET n.lastSeen = datetime()` runs through the write entry point
- **THEN** a `(v:alice v:lastSeen …)` statement is asserted, and the volatile entry is unchanged

### Requirement: Built-in procedures
`CALL db.labels()`, `CALL db.relationshipTypes()` and `CALL db.propertyKeys()`, with optional `YIELD`, SHALL return the distinct resolved names used by visible statements in the query's view, excluding `sys:` names. Calling any other procedure MUST fail with `Unsupported` naming it.

#### Scenario: db.labels
- **WHEN** nodes labelled `Person` and `Company` exist and `CALL db.labels() YIELD label RETURN label ORDER BY label` runs
- **THEN** the rows are `"Company"` and `"Person"`

#### Scenario: Unknown procedure
- **WHEN** `CALL dbms.components()` is compiled
- **THEN** it fails with `Unsupported` naming `dbms.components`

### Requirement: Unsupported openCypher features
The system SHALL parse the full openCypher grammar, and it MUST reject constructs outside the v1 subset with an `Unsupported` error that names the feature, before executing anything. The rejected constructs are: `FOREACH`, `LOAD CSV`, `CALL { … } IN TRANSACTIONS`, `USE <graph name>`, schema commands (`CREATE INDEX`, `CREATE CONSTRAINT`, `DROP …`, `SHOW …`), quantified path patterns (`{m,n}`, `+`, `*` quantifiers on parenthesised patterns), GQL path modes (`WALK`, `TRAIL`, `SIMPLE`, `ACYCLIC`), label expressions using `!`, `&` or `%`, and dynamic labels or types.

#### Scenario: FOREACH rejected
- **WHEN** `MATCH (n) FOREACH (x IN [1] | SET n.a = x)` is compiled
- **THEN** it fails with `Unsupported` naming `FOREACH`

#### Scenario: LOAD CSV rejected
- **WHEN** `LOAD CSV FROM 'file:///x.csv' AS row RETURN row` is compiled
- **THEN** it fails with `Unsupported` naming `LOAD CSV`

#### Scenario: Negated label rejected
- **WHEN** `MATCH (n:!Person) RETURN n` is compiled
- **THEN** it fails with `Unsupported` naming the label expression

### Requirement: Parse errors with spans
Text that is not valid openCypher, or not valid in a Tiramemsu extension, MUST fail with a `Parse` error that carries the Cypher dialect, the byte span of the offending input in the original query text, and a message. Nothing SHALL be executed.

#### Scenario: Unclosed parenthesis
- **WHEN** `MATCH (n:Person RETURN n` is compiled
- **THEN** it fails with a `Parse` error whose span starts at byte 6, the unclosed `(`

#### Scenario: Span refers to original text after extensions
- **WHEN** `USE AS OF 3 MATCH (n RETURN n` is compiled
- **THEN** the `Parse` error span points into the original text at the unclosed `(`, byte offset 18

### Requirement: Compile-time semantic errors
The system MUST reject these with a `Parse` error spanning the offending element, before execution: an undefined variable; a variable used with conflicting kinds, other than a relationship variable in node position as defined in `cypher-dual-view`; an aggregate in a disallowed position; a missing parameter; and `UNION` column mismatch.

#### Scenario: Node variable used as a relationship
- **WHEN** `MATCH (n) MATCH ()-[n]->() RETURN n` is compiled
- **THEN** it fails with a `Parse` error spanning the second `n`

#### Scenario: Undefined variable
- **WHEN** `MATCH (n) RETURN m` is compiled
- **THEN** it fails with a `Parse` error spanning `m`

### Requirement: Runtime evaluation errors
An expression that openCypher defines as a runtime error, such as arithmetic on incompatible types (`'a' - 1`), integer division or modulo by zero, or a property value of an unstorable type, MUST fail the whole query with an `Eval` error that carries a message. Conversions that openCypher defines as returning `null` (for example `toInteger('x')`) SHALL return `null`.

#### Scenario: Type error in arithmetic
- **WHEN** `RETURN 'a' - 1` runs
- **THEN** it fails with an `Eval` error

#### Scenario: Integer division by zero
- **WHEN** `RETURN 1 / 0` runs
- **THEN** it fails with an `Eval` error

#### Scenario: Lenient conversion
- **WHEN** `RETURN toInteger('x') AS v` runs
- **THEN** `v` is `null`
