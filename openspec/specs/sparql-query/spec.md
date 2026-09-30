# sparql-query Specification

## Purpose
Lets applications and agents read the Tiramemsu store with standard SPARQL 1.1 queries. It defines the query forms, the supported algebra and functions, how SPARQL semantics apply to a store of addressable triples, the errors for text outside the v1 subset, and the shape and serialisation of results.

## Requirements

### Requirement: SPARQL queries run against a view

The system SHALL accept SPARQL query text on any view (current, as of a transaction or instant, history, valid at a time, or speculative) and evaluate it against exactly the statements that view makes visible. A query SHALL never modify the store. Every example in these specs assumes that the prefixes of the "Predeclared prefixes" requirement are available and that `v:` is the default `@vocab` namespace `urn:tiramemsu:v:`.

#### Scenario: Current view sees live statements only
- **WHEN** `(v:alice v:worksAt v:acme)` was asserted in tx 1 and retracted in tx 2, and `SELECT ?c WHERE { v:alice v:worksAt ?c }` is run on the current view
- **THEN** the result has the single variable `c` and zero rows

#### Scenario: Historical view sees the past
- **WHEN** the same data is queried with the same text on the view as of tx 1
- **THEN** the result has exactly one row with `c = v:acme`

#### Scenario: Speculative view sees uncommitted state
- **WHEN** inside a speculative transaction that asserts `(v:bob v:worksAt v:initech)` the query `ASK { v:bob v:worksAt v:initech }` is run on the speculative view
- **THEN** the answer is `true`, and after the speculation ends the same query on the current view answers `false`

### Requirement: Supported query forms

The system SHALL support the `SELECT`, `ASK` and `CONSTRUCT` query forms. `SELECT` SHALL support explicit variable lists, `SELECT *`, projected expressions `(expr AS ?v)`, `DISTINCT` and `REDUCED`. `SELECT *` SHALL project every in-scope variable of the pattern. It SHALL NOT project internal variables that the system introduces for blank nodes, triple terms or reifiers. `ASK` SHALL return a boolean that is true exactly when the pattern has at least one solution. The `DESCRIBE` form SHALL fail with `Unsupported { feature: "DESCRIBE" }`.

#### Scenario: SELECT with projected expression
- **WHEN** `(v:alice v:age 41)` is live and `SELECT ?p ((?a + 1) AS ?next) WHERE { ?p v:age ?a }` is run
- **THEN** the result variables are `p, next` in that order, and the single row is `p = v:alice`, `next = 42` (an `xsd:integer`)

#### Scenario: SELECT star hides internal variables
- **WHEN** `SELECT * WHERE { ?s v:worksAt [] }` is run over one live `v:worksAt` statement
- **THEN** the result variables are exactly `s`, with no variable for the blank node

#### Scenario: ASK true and false
- **WHEN** `ASK { v:alice v:worksAt v:acme }` is run while that triple is live, and again after it is retracted
- **THEN** the first answer is `true` and the second is `false`

#### Scenario: DESCRIBE is unsupported
- **WHEN** `DESCRIBE v:alice` is submitted
- **THEN** the request fails with `Unsupported { feature: "DESCRIBE" }` and no rows are produced

### Requirement: Basic graph patterns and joins

The system SHALL evaluate a basic graph pattern as the join of its triple patterns, binding shared variables to equal values. Constant IRIs and literals SHALL match only statements that hold exactly that value. A constant that never occurs in the store SHALL make its pattern produce no solutions, and SHALL NOT be an error.

#### Scenario: Two-pattern join
- **WHEN** `(v:alice v:worksAt v:acme)`, `(v:acme v:locatedIn v:berlin)` and `(v:bob v:worksAt v:initech)` are live, and `SELECT ?p ?city WHERE { ?p v:worksAt ?c . ?c v:locatedIn ?city }` is run
- **THEN** exactly one row is returned: `p = v:alice`, `city = v:berlin`

#### Scenario: Unknown constant yields empty result
- **WHEN** `SELECT ?o WHERE { v:neverSeen v:worksAt ?o }` is run and `v:neverSeen` was never stored
- **THEN** the result has zero rows and no error

#### Scenario: Variable in predicate position
- **WHEN** `(v:alice v:worksAt v:acme)` and `(v:alice v:age 41)` are live and `SELECT ?p WHERE { v:alice ?p ?o } ORDER BY ?p` is run
- **THEN** the rows are `v:age` then `v:worksAt`

### Requirement: Set-of-triples semantics over eids

A triple pattern whose statement identity is not bound SHALL match each distinct `(s, p, o)` visible in the view once, however many visible eids carry that content: parallel statements, several episodes with different valid times, or a statement and its supersede replacement seen together under history. When a reifier binds the eid, each visible eid SHALL match separately. Solution multiplicity SHALL otherwise follow SPARQL 1.1 bag semantics, so joins and projections without `DISTINCT` may still repeat rows.

#### Scenario: Two episodes show as one triple
- **WHEN** `(v:alice v:worksAt v:acme)` is live twice, with valid times `[2020-01-01, 2022-01-01)` and `[2024-01-01, unbounded)`, and `SELECT ?c WHERE { v:alice v:worksAt ?c }` is run
- **THEN** exactly one row `c = v:acme` is returned

#### Scenario: Parallel edges created by Cypher show as one triple
- **WHEN** `(v:alice v:called v:bob)` was created twice through the create operation, giving two live eids, and `SELECT (COUNT(*) AS ?n) WHERE { v:alice v:called v:bob }` is run
- **THEN** the single row has `n = 1`

#### Scenario: Binding the eid exposes every occurrence
- **WHEN** the same data is queried with `SELECT ?r WHERE { v:alice v:called v:bob ~ ?r }`
- **THEN** two rows are returned, one per live eid

#### Scenario: Projection keeps bag semantics
- **WHEN** `(v:alice v:knows v:bob)` and `(v:alice v:knows v:carol)` are live and `SELECT ?s WHERE { ?s v:knows ?o }` is run
- **THEN** two rows with `s = v:alice` are returned, and with `SELECT DISTINCT ?s` one row is returned

### Requirement: Homomorphic matching

Two triple patterns in one query SHALL be allowed to match the same statement. Distinct variables SHALL be allowed to bind the same value, following SPARQL homomorphism semantics.

#### Scenario: Same statement matched by two patterns
- **WHEN** only `(v:alice v:knows v:bob)` is live and `SELECT ?a ?c WHERE { ?a v:knows ?b . ?c v:knows ?d }` is run
- **THEN** exactly one row is returned: `a = v:alice`, `c = v:alice`

### Requirement: OPTIONAL

The system SHALL evaluate `OPTIONAL { … }` as a left join. A left solution without a compatible right solution SHALL be kept, and the right-only variables SHALL be unbound. A `FILTER` directly inside the `OPTIONAL` group SHALL be part of the join condition, so a failing condition keeps the left row with unbound right variables.

#### Scenario: Missing optional value is unbound
- **WHEN** `(v:alice v:age 41)` and `(v:bob v:name "Bob")` are live and `SELECT ?p ?age WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?age } }` is run
- **THEN** one row is returned with `p = v:bob` and `age` unbound

#### Scenario: Filter inside OPTIONAL is a join condition
- **WHEN** `(v:alice v:name "Alice")` and `(v:alice v:age 41)` are live and `SELECT ?p ?age WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?age FILTER(?age > 50) } }` is run
- **THEN** one row is returned with `p = v:alice` and `age` unbound

### Requirement: FILTER and expression errors

The system SHALL keep a solution in `FILTER(expr)` only when the effective boolean value of `expr` is true. An expression that raises an error, including a type error or a reference to an unbound variable, SHALL make the filter false for that solution and SHALL NOT fail the query. Errors SHALL propagate through `!` and the other operators as defined by SPARQL 1.1, with `||` and `&&` following SPARQL's three-valued error rules. Numeric comparison and equality SHALL compare by numeric value across `xsd:integer`, `xsd:decimal` and `xsd:double`.

#### Scenario: Type error removes the row
- **WHEN** `(v:alice v:age 41)` and `(v:bob v:age "unknown")` are live and `SELECT ?p WHERE { ?p v:age ?a FILTER(?a > 30) }` is run
- **THEN** exactly one row `p = v:alice` is returned and no error is raised

#### Scenario: Negating an error still removes the row
- **WHEN** the same data is queried with `SELECT ?p WHERE { ?p v:age ?a FILTER(!(?a > 30)) }`
- **THEN** zero rows are returned

#### Scenario: Unbound variable in filter
- **WHEN** `SELECT ?p WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } FILTER(?a < 100) }` is run over a person with a name and no age
- **THEN** that person is not returned

#### Scenario: Error rescued by OR
- **WHEN** `SELECT ?p WHERE { ?p v:age ?a FILTER(?a > 30 || true) }` is run over the data of the first scenario
- **THEN** both `v:alice` and `v:bob` are returned

#### Scenario: Numeric equality across datatypes
- **WHEN** `(v:alice v:age 41)` is live and `ASK { v:alice v:age ?a FILTER(?a = 41.0) }` is run
- **THEN** the answer is `true`

### Requirement: UNION

The system SHALL evaluate `{ A } UNION { B }` as the bag union of the solutions of both branches. Variables that are bound in only one branch SHALL be unbound in rows coming from the other branch.

#### Scenario: Union of two predicates
- **WHEN** `(v:alice v:email "a@x.org")` and `(v:bob v:phone "123")` are live and `SELECT ?p ?e ?t WHERE { { ?p v:email ?e } UNION { ?p v:phone ?t } }` is run
- **THEN** two rows are returned: `(v:alice, "a@x.org", unbound)` and `(v:bob, unbound, "123")`

### Requirement: MINUS and EXISTS

The system SHALL evaluate `MINUS { … }` with SPARQL 1.1 semantics. A left solution SHALL be removed only when a right solution is compatible with it and the two share at least one bound variable. `FILTER EXISTS { … }` and `FILTER NOT EXISTS { … }` SHALL test the inner pattern with the current solution's bindings substituted.

#### Scenario: MINUS removes matching subjects
- **WHEN** `(v:alice a v:Person)`, `(v:bob a v:Person)` and `(v:bob v:banned true)` are live and `SELECT ?p WHERE { ?p a v:Person MINUS { ?p v:banned true } }` is run
- **THEN** exactly one row `p = v:alice` is returned

#### Scenario: MINUS without shared variables removes nothing
- **WHEN** `SELECT ?p WHERE { ?p a v:Person MINUS { ?x v:banned true } }` is run over the same data
- **THEN** both `v:alice` and `v:bob` are returned

#### Scenario: NOT EXISTS is correlated
- **WHEN** `SELECT ?p WHERE { ?p a v:Person FILTER NOT EXISTS { ?p v:banned true } }` is run over the same data
- **THEN** exactly one row `p = v:alice` is returned

### Requirement: BIND and VALUES

The system SHALL evaluate `BIND(expr AS ?v)` by extending each solution with the value of `expr`. When `expr` raises an error, `?v` SHALL be left unbound and the solution SHALL be kept. Binding a variable that is already in scope SHALL be rejected as a parse error, as SPARQL 1.1 requires. The system SHALL evaluate `VALUES` blocks, both inline and trailing, as inline data joined with the pattern. `UNDEF` SHALL leave the variable unbound in that row.

#### Scenario: BIND computes a value
- **WHEN** `(v:alice v:age 41)` is live and `SELECT ?y WHERE { v:alice v:age ?a BIND(2026 - ?a AS ?y) }` is run
- **THEN** the single row has `y = 1985`

#### Scenario: BIND error leaves the variable unbound
- **WHEN** `(v:bob v:age "unknown")` is live and `SELECT ?p ?y WHERE { ?p v:age ?a BIND(?a * 2 AS ?y) }` is run
- **THEN** one row is returned with `p = v:bob` and `y` unbound

#### Scenario: BIND over an in-scope variable is rejected
- **WHEN** `SELECT ?a WHERE { v:alice v:age ?a BIND(1 AS ?a) }` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL

#### Scenario: VALUES restricts solutions
- **WHEN** `(v:alice v:age 41)` and `(v:bob v:age 30)` are live and `SELECT ?p ?a WHERE { VALUES ?p { v:bob v:zoe } ?p v:age ?a }` is run
- **THEN** exactly one row `(v:bob, 30)` is returned

#### Scenario: UNDEF in VALUES
- **WHEN** `SELECT ?p ?a WHERE { ?p v:age ?a } VALUES (?p ?a) { (UNDEF 30) }` is run over the same data
- **THEN** exactly one row `(v:bob, 30)` is returned

### Requirement: Aggregates and grouping

The system SHALL support `GROUP BY` (over variables and expressions), `HAVING`, and the aggregates `COUNT` (including `COUNT(*)` and `COUNT(DISTINCT …)`), `SUM`, `AVG`, `MIN`, `MAX`, `SAMPLE` and `GROUP_CONCAT` (with and without `SEPARATOR`), each with optional `DISTINCT`. An aggregate query without `GROUP BY` SHALL form one group, even over zero solutions. Custom aggregate functions SHALL fail with `Unsupported { feature: "custom aggregate" }`.

#### Scenario: Count per group
- **WHEN** `(v:alice v:worksAt v:acme)`, `(v:bob v:worksAt v:acme)` and `(v:carol v:worksAt v:initech)` are live and `SELECT ?c (COUNT(?p) AS ?n) WHERE { ?p v:worksAt ?c } GROUP BY ?c ORDER BY ?c` is run
- **THEN** the rows are `(v:acme, 2)` and `(v:initech, 1)`

#### Scenario: HAVING filters groups
- **WHEN** the same query adds `HAVING (COUNT(?p) > 1)`
- **THEN** only the row `(v:acme, 2)` is returned

#### Scenario: Count over no solutions
- **WHEN** `SELECT (COUNT(*) AS ?n) WHERE { ?p v:neverUsed ?o }` is run
- **THEN** one row with `n = 0` is returned

#### Scenario: GROUP_CONCAT with separator
- **WHEN** `SELECT (GROUP_CONCAT(?n; SEPARATOR="|") AS ?all) WHERE { VALUES ?n { "a" "b" } }` is run
- **THEN** one row is returned whose `all` is `"a|b"` or `"b|a"`

### Requirement: Subqueries

The system SHALL evaluate nested `SELECT` subqueries, including their own `DISTINCT`, `GROUP BY`, `ORDER BY`, `LIMIT` and `OFFSET`. Only the variables projected by the subquery SHALL be visible to the enclosing pattern.

#### Scenario: Top-N per subquery
- **WHEN** `(v:alice v:age 41)`, `(v:bob v:age 30)` and `(v:carol v:age 25)` are live and `SELECT ?p ?n WHERE { { SELECT ?p WHERE { ?p v:age ?a } ORDER BY DESC(?a) LIMIT 2 } OPTIONAL { ?p v:name ?n } }` is run
- **THEN** exactly the rows for `v:alice` and `v:bob` are returned

#### Scenario: Inner variables are hidden
- **WHEN** `SELECT ?a WHERE { { SELECT ?p WHERE { ?p v:age ?a } } }` is run
- **THEN** every row has `a` unbound

### Requirement: Solution modifiers

The system SHALL apply `ORDER BY` (with `ASC`/`DESC` and expression keys), `LIMIT` and `OFFSET` with SPARQL 1.1 semantics. Ordering SHALL compare values, not internal identifiers: numbers numerically, strings by Unicode code point, date-times chronologically. Terms of different kinds SHALL order as unbound < blank node < IRI < literal. A query that combines `DISTINCT` with an `ORDER BY` key over a variable that is not projected SHALL fail with `Unsupported { feature: "ORDER BY non-projected variable with DISTINCT" }`.

#### Scenario: Numeric order is by value
- **WHEN** `(v:a v:rank 9)` and `(v:b v:rank 10)` are live and `SELECT ?s WHERE { ?s v:rank ?r } ORDER BY ?r` is run
- **THEN** the rows are `v:a` then `v:b`

#### Scenario: Descending string order with limit and offset
- **WHEN** names `"ann"`, `"bob"`, `"cy"` and `"dee"` are live and `SELECT ?n WHERE { ?p v:name ?n } ORDER BY DESC(?n) LIMIT 2 OFFSET 1` is run
- **THEN** the rows are `"cy"` then `"bob"`

#### Scenario: Unbound sorts first
- **WHEN** `SELECT ?p ?a WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } } ORDER BY ?a` is run over one person with an age and one without
- **THEN** the person without an age comes first

### Requirement: Built-in functions

The system SHALL support these SPARQL 1.1 operators and functions: logical, comparison and arithmetic operators, `IN`/`NOT IN`, `BOUND`, `IF`, `COALESCE`, `sameTerm`, `isIRI`/`isURI`, `isBlank`, `isLiteral`, `isNumeric`, `STR`, `LANG`, `LANGMATCHES`, `DATATYPE`, `IRI`/`URI`, `STRDT`, `STRLANG`, `STRLEN`, `SUBSTR`, `UCASE`, `LCASE`, `STRSTARTS`, `STRENDS`, `CONTAINS`, `STRBEFORE`, `STRAFTER`, `CONCAT`, `ENCODE_FOR_URI`, `REGEX`, `REPLACE`, `ABS`, `CEIL`, `FLOOR`, `ROUND`, `YEAR`, `MONTH`, `DAY`, `HOURS`, `MINUTES`, `SECONDS`, `TIMEZONE`, `TZ`, `NOW`, and XSD constructor casts to `xsd:string`, `xsd:integer`, `xsd:decimal`, `xsd:double`, `xsd:boolean`, `xsd:date` and `xsd:dateTime`. Any other function, including `RAND`, `BNODE`, `UUID`, `STRUUID`, `MD5`, `SHA1`, `SHA256`, `SHA384`, `SHA512`, the SPARQL 1.2 language-direction functions and any custom function IRI, SHALL fail with `Unsupported { feature }`, where `feature` names the function. `YEAR` to `SECONDS` SHALL read the local time in the date-time's stored offset. `TZ` SHALL return the stored offset as a plain string (`"Z"` for a zero offset, `""` without a timezone). `TIMEZONE` SHALL return it as an `xsd:dayTimeDuration` and SHALL raise an error for a date-time without a timezone, as SPARQL 1.1 requires.

#### Scenario: String functions
- **WHEN** `(v:alice v:name "Alice Smith")` is live and `SELECT (UCASE(STRBEFORE(?n, " ")) AS ?f) WHERE { v:alice v:name ?n }` is run
- **THEN** the single row has `f = "ALICE"`

#### Scenario: Unsupported function is named
- **WHEN** `SELECT (MD5(?n) AS ?h) WHERE { ?p v:name ?n }` is submitted
- **THEN** the request fails with `Unsupported { feature: "MD5" }` before any row is produced

#### Scenario: Custom function is unsupported
- **WHEN** `SELECT ?x WHERE { ?x v:age ?a FILTER(<http://example.org/fn>(?a)) }` is submitted
- **THEN** the request fails with `Unsupported` whose feature names `http://example.org/fn`

### Requirement: Literal canonicalisation is visible in queries

Query constants SHALL be encoded with the store's canonical value encoding, so matching and joins compare encoded terms. Numbers and booleans SHALL collapse to their value: an `xsd:integer` SHALL match by value whatever its lexical form, and `sameTerm` SHALL follow the same encoding, which is a deliberate deviation from RDF 1.1 term identity. An `xsd:dateTime` SHALL keep its timezone offset, or the absence of one, as part of the term, with millisecond precision. Two date-times SHALL be the same term, for `sameTerm`, for constants in triple patterns and for joins on a shared variable, only when both the instant and the offset match, as in RDF. The comparison operators (`=`, `!=`, `<`, `>`, `<=`, `>=`) and `ORDER BY` SHALL compare date-times by instant, and a date-time without a timezone SHALL be compared as if it were UTC. Language tags SHALL match case-insensitively and be returned in lower case. Values SHALL be returned in canonical lexical form, except that a date-time SHALL be returned with the offset it was stored with.

#### Scenario: Integer lexical forms are equal
- **WHEN** `(v:alice v:age 1)` is live and `ASK { v:alice v:age "01"^^xsd:integer }` is run
- **THEN** the answer is `true`

#### Scenario: sameTerm follows canonical encoding
- **WHEN** `ASK { FILTER(sameTerm("01"^^xsd:integer, 1)) }` is run
- **THEN** the answer is `true`

#### Scenario: Date-time offsets are distinct terms
- **WHEN** `ASK { FILTER(sameTerm("2026-03-01T12:00:00+02:00"^^xsd:dateTime, "2026-03-01T10:00:00Z"^^xsd:dateTime)) }` is run, and then the same query with `=` in place of `sameTerm`
- **THEN** the first answer is `false` and the second is `true`

#### Scenario: Pattern constant matches by term, FILTER by instant
- **WHEN** `(v:e v:at "2026-09-01T12:00:00Z"^^xsd:dateTime)` is live, and `ASK { v:e v:at "2026-09-01T14:00:00+02:00"^^xsd:dateTime }` and `ASK { v:e v:at ?t FILTER(?t = "2026-09-01T14:00:00+02:00"^^xsd:dateTime) }` are run
- **THEN** the first answer is `false` and the second is `true`

#### Scenario: Date-time keeps its offset in results
- **WHEN** `(v:meeting v:at "2026-09-01T09:00:00+03:00"^^xsd:dateTime)` is inserted and `SELECT ?t WHERE { v:meeting v:at ?t }` is run
- **THEN** one row is returned whose `t` is `"2026-09-01T09:00:00+03:00"^^xsd:dateTime`

#### Scenario: Ordering compares instants across offsets
- **WHEN** `(v:a v:at "2026-09-01T12:00:00+02:00"^^xsd:dateTime)` and `(v:b v:at "2026-09-01T11:00:00Z"^^xsd:dateTime)` are live and `SELECT ?s WHERE { ?s v:at ?t } ORDER BY ?t` is run
- **THEN** the rows are `v:a` (10:00 UTC) then `v:b` (11:00 UTC)

#### Scenario: TIMEZONE and TZ read the stored offset
- **WHEN** `(v:a v:at "2026-09-01T12:00:00+02:00"^^xsd:dateTime)` and `(v:b v:at "2026-09-01T12:00:00"^^xsd:dateTime)` are live, and `SELECT ?s (TZ(?t) AS ?z) (TIMEZONE(?t) AS ?d) WHERE { ?s v:at ?t }` is run
- **THEN** the row for `v:a` has `z = "+02:00"` and `d = "PT2H"^^xsd:dayTimeDuration`, and the row for `v:b` has `z = ""` and `d` unbound, because `TIMEZONE` raises an error for a date-time without a timezone

#### Scenario: Language tag case
- **WHEN** `(v:alice v:greeting "Hallo"@DE)` was inserted and `SELECT (LANG(?g) AS ?l) WHERE { v:alice v:greeting ?g FILTER(?g = "Hallo"@de) }` is run
- **THEN** one row with `l = "de"` is returned

### Requirement: Blank nodes and skolem IRIs in queries

Blank nodes in query patterns SHALL act as variables that are not projected. A skolem IRI written in a query (`urn:tiramemsu:node:<n>`, `urn:tiramemsu:bnode:<n>`, `urn:tiramemsu:stmt:<n>` or `urn:tiramemsu:tx:<t>`) SHALL denote the same anonymous node, blank node, statement or transaction that the system renders with that IRI, so values round-trip exactly between results and later queries.

#### Scenario: Blank node label is not projected
- **WHEN** `SELECT ?p WHERE { ?p v:worksAt _:c . _:c v:locatedIn v:berlin }` is run over data where alice works at a Berlin company
- **THEN** one row with `p = v:alice` is returned, and the result has no variable for `_:c`

#### Scenario: Anonymous node round trip
- **WHEN** a node created without an IRI is returned by a query as `urn:tiramemsu:node:7`, and `SELECT ?n WHERE { <urn:tiramemsu:node:7> v:name ?n }` is then run
- **THEN** the name of that same node is returned

### Requirement: Property paths before the path engine

The system SHALL parse every SPARQL 1.1 property path. A path that is a single IRI SHALL be evaluated as a triple pattern. A path that is the inverse `^iri` of a single IRI SHALL be evaluated as the triple pattern with subject and object swapped. Every other path form (sequence `/`, alternative `|`, `*`, `+`, `?`, and negated property sets `!`) SHALL fail with `Unsupported { feature: "property path" }` before execution. This requirement is interim: the path engine change replaces it.

#### Scenario: Inverse single predicate
- **WHEN** `(v:alice v:worksAt v:acme)` is live and `SELECT ?p WHERE { v:acme ^v:worksAt ?p }` is run
- **THEN** one row with `p = v:alice` is returned

#### Scenario: Transitive path is unsupported for now
- **WHEN** `SELECT ?x WHERE { v:alice v:knows+ ?x }` is submitted
- **THEN** the request fails with `Unsupported { feature: "property path" }`

#### Scenario: Sequence path is unsupported for now
- **WHEN** `SELECT ?city WHERE { v:alice v:worksAt/v:locatedIn ?city }` is submitted
- **THEN** the request fails with `Unsupported { feature: "property path" }`

### Requirement: Unsupported features fail before execution

Constructs outside the v1 subset SHALL be rejected after parsing and before any SQL is executed, with `Unsupported { feature }` naming the construct. This SHALL cover at least: `DESCRIBE` (`"DESCRIBE"`), `SERVICE` with a variable or with an IRI that is not a time IRI, that is federation (`"SERVICE"`), `GRAPH` with a variable (`"GRAPH variable"`), `FROM`, `FROM NAMED` or `GRAPH` with an IRI outside the `tm:` namespace (`"named graph"`), custom aggregates, the functions listed as unsupported, and the property paths of the interim path requirement. A rejected query SHALL return no partial results. `SERVICE` with a `tm:` time IRI is not federation: it is the per-group time scope of the temporal dataset capability. `GRAPH` with a `tm:` IRI is a `Parse` error that names `SERVICE`, as that capability specifies.

#### Scenario: Federated SERVICE is rejected
- **WHEN** `SELECT * WHERE { SERVICE <http://dbpedia.org/sparql> { ?s ?p ?o } }` is submitted
- **THEN** the request fails with `Unsupported { feature: "SERVICE" }`

#### Scenario: GRAPH variable is rejected
- **WHEN** `SELECT ?g WHERE { GRAPH ?g { ?s ?p ?o } }` is submitted
- **THEN** the request fails with `Unsupported { feature: "GRAPH variable" }`

### Requirement: Parse errors report position

Text that is not valid SPARQL 1.1 (with the SPARQL 1.2 triple-term, reifier and annotation syntax) SHALL fail with a `Parse` error whose dialect is SPARQL. The error SHALL carry the 1-based line and column, and the byte offset, at which parsing failed, together with a message that says what was expected. No part of the store SHALL be read or written.

#### Scenario: Syntax error location
- **WHEN** the text `SELECT ?s WHERE {\n  ?s v:p ?o\n  FILTER(\n}` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL whose position is on line 4, column 1

#### Scenario: Undeclared prefix
- **WHEN** `SELECT ?s WHERE { ?s ex:p ?o }` is submitted and `ex:` is neither declared in the query nor predeclared
- **THEN** the request fails with a `Parse` error of dialect SPARQL whose message says that a prefix was not found

### Requirement: Predeclared prefixes

The system SHALL predeclare the prefixes `rdf:`, `rdfs:`, `xsd:`, `sys:` (`urn:tiramemsu:sys:`), `tm:` (`urn:tiramemsu:tm:`), `v:` (the database's configured `@vocab` IRI) and every entry of the database's prefix table. A `PREFIX` declaration in the query SHALL override a predeclared prefix of the same name for that query.

#### Scenario: Default vocabulary prefix
- **WHEN** the database `@vocab` is the default and `SELECT ?c WHERE { <urn:tiramemsu:v:alice> v:worksAt ?c }` is run without any `PREFIX` declaration
- **THEN** the query parses and `v:worksAt` denotes `urn:tiramemsu:v:worksAt`

#### Scenario: Database prefix table
- **WHEN** the prefix table maps `schema` to `https://schema.org/` and `SELECT ?n WHERE { ?p schema:name ?n }` is run
- **THEN** `schema:name` denotes `https://schema.org/name`

#### Scenario: Query prefix overrides
- **WHEN** a query declares `PREFIX v: <http://example.org/>` and uses `v:x`
- **THEN** `v:x` denotes `http://example.org/x` for that query only

### Requirement: Result terms

Every solution value SHALL be an RDF term. IRIs SHALL be returned as IRIs. Anonymous nodes, blank nodes, statements (eids) and transactions SHALL be returned as the skolem IRIs `urn:tiramemsu:node:<n>`, `urn:tiramemsu:bnode:<n>`, `urn:tiramemsu:stmt:<n>` and `urn:tiramemsu:tx:<t>`, where `<n>` and `<t>` are unsigned decimal numbers. Literals SHALL be returned as follows: integers as `xsd:integer`, booleans as `xsd:boolean` (`true`/`false`), date-times as `xsd:dateTime` written in their stored offset (the local time for that offset followed by `±hh:mm`, `Z` for a zero offset, and no suffix when stored without a timezone) with fractional seconds only when the milliseconds are non-zero, dates as `xsd:date`, plain strings without a datatype, language strings with their lower-cased tag, and every other literal with its stored lexical form and datatype IRI.

#### Scenario: Eid rendered as statement IRI
- **WHEN** `(v:alice v:worksAt v:acme)` has eid number 12 and `SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }` is run
- **THEN** the single row has `r = <urn:tiramemsu:stmt:12>`

#### Scenario: Date-time rendering
- **WHEN** a value stored as the instant 2026-09-01T12:00:00.250Z is returned
- **THEN** it is the literal `"2026-09-01T12:00:00.250Z"^^xsd:dateTime`, and the instant 2026-09-01T12:00:00.000Z is returned as `"2026-09-01T12:00:00Z"^^xsd:dateTime`

#### Scenario: Date-time rendering keeps the offset
- **WHEN** values inserted as `"2026-09-01T14:00:00.000+02:00"^^xsd:dateTime`, `"2026-09-01T12:00:00+00:00"^^xsd:dateTime` and `"2026-09-01T12:00:00"^^xsd:dateTime` are returned
- **THEN** they are `"2026-09-01T14:00:00+02:00"^^xsd:dateTime`, `"2026-09-01T12:00:00Z"^^xsd:dateTime` and `"2026-09-01T12:00:00"^^xsd:dateTime`

### Requirement: SPARQL JSON results

`SELECT` and `ASK` results SHALL be serialisable as SPARQL 1.1 Query Results JSON. For `SELECT`, `head.vars` SHALL list the projected variables in projection order, and each row SHALL be an object in `results.bindings` that omits unbound variables. IRIs and skolem IRIs SHALL use `"type": "uri"`. Literals SHALL use `"type": "literal"`, with `"xml:lang"` for language strings and `"datatype"` for every typed literal except plain strings. For `ASK`, the document SHALL be `{"head": {}, "boolean": <answer>}`. Row order SHALL be preserved when the query has `ORDER BY`.

#### Scenario: SELECT JSON document
- **WHEN** `SELECT ?p ?age WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?age } }` returns one row with `p = v:bob` and `age` unbound, and the result is serialised
- **THEN** the output is `{"head":{"vars":["p","age"]},"results":{"bindings":[{"p":{"type":"uri","value":"urn:tiramemsu:v:bob"}}]}}`, ignoring whitespace

#### Scenario: Typed literal JSON
- **WHEN** a row binds `a` to the integer 41 and is serialised
- **THEN** the binding is `{"type":"literal","value":"41","datatype":"http://www.w3.org/2001/XMLSchema#integer"}`

#### Scenario: ASK JSON document
- **WHEN** an `ASK` query answers true and is serialised
- **THEN** the output is `{"head":{},"boolean":true}`, ignoring whitespace

### Requirement: CONSTRUCT results

`CONSTRUCT` SHALL return the set of RDF triples obtained by instantiating the template once per solution. Template triples with an unbound variable or an invalid term position (for example a literal subject) SHALL be skipped for that solution. Blank nodes in the template SHALL be fresh for each solution. Duplicate triples SHALL appear once. The result SHALL be serialisable as N-Triples, using RDF 1.2 N-Triples when a triple term occurs. `CONSTRUCT WHERE { … }` SHALL be supported.

#### Scenario: CONSTRUCT maps predicates
- **WHEN** `(v:alice v:worksAt v:acme)` is live and `CONSTRUCT { ?c v:employs ?p } WHERE { ?p v:worksAt ?c }` is run
- **THEN** the result is exactly the triple `<urn:tiramemsu:v:acme> <urn:tiramemsu:v:employs> <urn:tiramemsu:v:alice>`

#### Scenario: Unbound template variable skipped
- **WHEN** `CONSTRUCT { ?p v:hasAge ?a } WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } }` runs over a person without an age
- **THEN** no triple is produced for that person and no error is raised

#### Scenario: Fresh blank node per solution
- **WHEN** `CONSTRUCT { ?p v:card [ v:name ?n ] } WHERE { ?p v:name ?n }` runs over two named people
- **THEN** four triples are produced, and the two card nodes are distinct blank nodes
