## ADDED Requirements

### Requirement: Allen relations over half-open valid intervals
The system SHALL define the thirteen Allen relations `before`, `after`, `meets`, `metBy`, `overlaps`, `overlappedBy`, `starts`, `startedBy`, `during`, `contains`, `finishes`, `finishedBy` and `equals` over half-open valid intervals `[from, to)`, where an unbounded start is −∞, an unbounded end is +∞, and two unbounded ends on the same side are equal. For any two proper intervals exactly one relation SHALL hold. `intersects` SHALL hold when the intervals share at least one instant, and SHALL be identical to the overlap test that idempotent assert uses.

#### Scenario: Meets is exact for half-open intervals
- **WHEN** e1 is valid [2020-01-01, 2022-01-01) and e2 is valid [2022-01-01, 2024-01-01)
- **THEN** e1 `meets` e2 holds, `intersects` does not, and `before` does not

#### Scenario: Unbounded ends
- **WHEN** e1 has no valid time and e2 is valid [2025-01-01, open)
- **THEN** e2 `finishes` e1 holds, because both ends are +∞ and e2 starts later than −∞

#### Scenario: Two always-valid statements
- **WHEN** e1 and e2 both have no valid time
- **THEN** e1 `equals` e2 holds

### Requirement: Interval operands
An operand SHALL be a statement, contributing its stored valid interval whatever the view; an interval value built from two bounds where an open end is written `tm:unbounded` in SPARQL or `null` in Cypher; or an instant (a date at midnight UTC, a date-time at its instant, a local date-time read as UTC), contributing the interval `[t, t + 1 ms)`. A non-statement node, a non-temporal literal, or an empty or inverted interval SHALL be a type error: in SPARQL a `FILTER` evaluates to false and a `BIND` leaves its variable unbound; in Cypher the query fails with `Eval`.

#### Scenario: Statement against a constant period
- **WHEN** e1 is valid [2025-03-01, 2025-09-01) and `SELECT ?r WHERE { ?s ?p ?o ~ ?r FILTER(tm:intersects(?r, tm:interval("2025-01-01"^^xsd:date, "2025-07-01"^^xsd:date))) }` runs
- **THEN** e1 is returned

#### Scenario: Open end in an interval value
- **WHEN** `SELECT ?r WHERE { ?s ?p ?o ~ ?r FILTER(tm:during(?r, tm:interval("2020-01-01"^^xsd:date, tm:unbounded))) }` runs over e1 valid [2021-01-01, 2022-01-01)
- **THEN** e1 is returned

#### Scenario: Inverted interval is a type error
- **WHEN** `SELECT ?x WHERE { BIND(tm:interval("2025-02-01"^^xsd:date, "2025-01-01"^^xsd:date) AS ?x) }` runs
- **THEN** one row is returned with `?x` unbound

### Requirement: Interval functions in SPARQL
SPARQL SHALL provide the boolean functions `tm:before`, `tm:after`, `tm:meets`, `tm:metBy`, `tm:overlaps`, `tm:overlappedBy`, `tm:starts`, `tm:startedBy`, `tm:during`, `tm:contains`, `tm:finishes`, `tm:finishedBy`, `tm:equals` and `tm:intersects` of two operands; `tm:allen(a, b)` returning the name of the relation that holds as a plain string; `tm:interval(from, to)` and `tm:validInterval(statement)` returning a `tm:Interval` literal of the form `"[from,to)"` with UTC instants and `-inf`/`+inf` for open ends. The functions SHALL lower to SQL comparisons on the operands' valid-time columns, and SHALL honor the pattern's view only in choosing which statements are matched, never in reading a matched statement's interval.

#### Scenario: Relation name
- **WHEN** e1 is valid [2020, 2022) and e2 is valid [2021, 2023), and `SELECT (tm:allen(?a, ?b) AS ?rel) WHERE { … }` binds `?a` to e1 and `?b` to e2
- **THEN** `?rel` is `"overlaps"`

#### Scenario: Interval value rendering
- **WHEN** `SELECT (tm:validInterval(?r) AS ?i) WHERE { … }` binds `?r` to a statement valid from 2025-01-01 with no end
- **THEN** `?i` is `"[2025-01-01T00:00:00.000Z,+inf)"^^tm:Interval`

### Requirement: Interval functions in Cypher
Cypher SHALL provide `tiramemsu.interval.before`, `.after`, `.meets`, `.metBy`, `.overlaps`, `.overlappedBy`, `.starts`, `.startedBy`, `.during`, `.contains`, `.finishes`, `.finishedBy`, `.equals` and `.intersects` of two operands, `tiramemsu.interval.relation(a, b)` returning the relation name, and `tiramemsu.interval(from, to)` returning the map `{from, to}`. A relationship or a statement in node form SHALL contribute its valid interval, a map with keys `from` and `to` SHALL be an interval value, and a `null` operand SHALL yield `null`.

#### Scenario: Consecutive jobs
- **WHEN** `(v:alice v:worksAt v:acme)` is valid [2020, 2022) and `(v:alice v:worksAt v:globex)` is valid [2022, open), and `MATCH (a)-[r1:worksAt]->(), (a)-[r2:worksAt]->() WHERE tiramemsu.interval.meets(r1, r2) RETURN r1, r2` runs
- **THEN** exactly the pair (acme job, globex job) is returned

#### Scenario: Null operand
- **WHEN** `RETURN tiramemsu.interval.before(null, datetime()) AS b` runs
- **THEN** `b` is `null`

### Requirement: Dialects agree on interval questions
The SPARQL functions, the Cypher functions and `Valid::relate` in Rust SHALL give the same answer for the same operands, so equivalent queries in both languages return the same statements.

#### Scenario: Same statements in both languages
- **WHEN** "statements whose valid interval intersects [2025-01-01, 2025-07-01)" is asked through SPARQL `tm:intersects` and Cypher `tiramemsu.interval.intersects` on the same data
- **THEN** both return the same set of statement ids
