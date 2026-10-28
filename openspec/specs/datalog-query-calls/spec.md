# datalog-query-calls Specification

## Purpose
Calling a Datalog goal from SPARQL and Cypher as a table that joins with the rest of the query, evaluated once per query under the query's snapshot and budget.

## Requirements

### Requirement: SPARQL goal service
A SPARQL query SHALL accept `SERVICE <urn:tiramemsu:dl:query> { [] dl:goal "body" ; (dl:program <iri> | dl:text "program text") ; dl:validFrom ?f ; dl:validTo ?t }`, with `dl:validFrom` and `dl:validTo` optional. Each variable of the goal SHALL bind the SPARQL variable of the same name; the service SHALL produce one solution per goal row and maximal valid interval, with `?f` and `?t` as `xsd:dateTime` (unbound for an open end). A missing or non-constant `dl:goal` or program, or both `dl:program` and `dl:text`, SHALL fail with `DatalogCallShape`.

#### Scenario: Goal joined with a pattern
- **WHEN** the `reports` program is stored as `ex:reporting` and the query is `SELECT ?X ?mail { SERVICE <urn:tiramemsu:dl:query> { [] dl:program ex:reporting ; dl:goal "reports(ex:alice, X)" } ?X ex:email ?mail }`
- **THEN** it returns one row per manager above alice who has an email

#### Scenario: Intervals as columns
- **WHEN** the same service has `dl:validFrom ?f ; dl:validTo ?t` and alice reports to bob only during `[2020, 2023)`
- **THEN** the row for bob has `?f = 2020-01-01T00:00:00Z` and `?t = 2023-01-01T00:00:00Z`

#### Scenario: Variable goal refused
- **WHEN** `dl:goal ?g` is a variable
- **THEN** the query fails with `DatalogCallShape`

### Requirement: Cypher goal procedure
Cypher SHALL accept `CALL tiramemsu.datalog.query(program, goal) YIELD …`, where `program` is a stored program's IRI string or a map `{text: "…"}` and `goal` a string, both constants or parameters. Its columns SHALL be the goal's variables plus `validFrom` and `validTo`, one row per goal row and maximal interval.

#### Scenario: Call then match
- **WHEN** the query is `CALL tiramemsu.datalog.query('http://example.org/reporting', 'reports(ex:alice, X)') YIELD X MATCH (m {iri: X})-[:email]->(e) RETURN X, e`
- **THEN** it returns the same pairs as the SPARQL scenario "Goal joined with a pattern"

#### Scenario: Parameters are constants
- **WHEN** the goal is passed as the parameter `$goal`
- **THEN** the call runs once with its value

### Requirement: Evaluated once per query
A goal call SHALL be evaluated once per query execution, in the query's snapshot and view, before or independently of the rows of the rest of the query, including inside `OPTIONAL`, `UNION`, subqueries and Cypher `OPTIONAL MATCH`. Explain SHALL show one call.

#### Scenario: Optional side runs once
- **WHEN** the call is inside `OPTIONAL { }` under a pattern with 1 000 rows
- **THEN** explain shows one evaluation of the goal, and the rows equal those of a query that joins against the call's rows computed separately

### Requirement: Same rows as the Rust call
The rows of a goal call SHALL equal the rows of `View::datalog(program, goal)` on the same view, on SQLite and on redb.

#### Scenario: Three surfaces agree
- **WHEN** the same goal runs through `View::datalog`, the SPARQL service and the Cypher procedure on SQLite and redb
- **THEN** all six return the same bindings and intervals

### Requirement: Budgets apply to calls
A goal call SHALL run under the query's budget: its deadline and cancellation SHALL stop the call and the query with the budget's error; the call's derived facts SHALL count against the operation's byte budget.

#### Scenario: Deadline inside a query
- **WHEN** a query with a call over a large closure runs under a 1 ms deadline
- **THEN** it fails with the deadline error

### Requirement: Feature gating for calls
Without the facade feature `datalog`, the SPARQL service and Cypher procedure SHALL parse and fail at execution with `Unsupported`. With `datalog` but without `exec`, `View::datalog` SHALL work and the query languages SHALL be absent as usual.

#### Scenario: Call without the feature
- **WHEN** the facade is built with `sparql` but not `datalog` and the service is used
- **THEN** the query fails with `Unsupported`

### Requirement: Provenance gap for calls
With query provenance on, rows that come from a goal call SHALL add `ProvenanceGap::DatalogCall`, so `provenance_complete()` is false for those queries; statements matched by the query's ordinary patterns SHALL still be cited.

#### Scenario: Provenance marks the call
- **WHEN** the scenario "Goal joined with a pattern" runs with provenance
- **THEN** the email statements are cited and the provenance gaps include `DatalogCall`
