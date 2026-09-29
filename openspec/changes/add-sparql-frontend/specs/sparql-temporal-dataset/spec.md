## Purpose

Lets SPARQL queries choose transaction time and valid time for a whole query or per pattern, through `tm:` time IRIs in the standard `FROM` and `GRAPH` clauses. It also lets queries read statement-time metadata through `tm:` virtual predicates, so "what did we know, and when" is plain SPARQL.

## ADDED Requirements

### Requirement: Time IRI grammar

The system SHALL recognise these time IRIs in `FROM`, `FROM NAMED`, `GRAPH`, `USING` and `USING NAMED`, where `urn:tiramemsu:tm:` is the `tm:` namespace:
- `urn:tiramemsu:tm:asOf/<t>`, where `<t>` is an unsigned decimal transaction number: the transaction-time view as of transaction `t`.
- `urn:tiramemsu:tm:asOf/<instant>`, where `<instant>` is an `xsd:dateTime` or `xsd:date` lexical form: the view as of the largest transaction whose instant is at or before that time. It is the empty view if no transaction is that old.
- `urn:tiramemsu:tm:validAt/<instant>`, with the same lexical forms: only statements whose valid-time interval `[v_from, v_to)` contains that time.
- `urn:tiramemsu:tm:history`: every statement ever stored, live or retracted.

A date SHALL mean 00:00:00 UTC of that day. A date-time without a timezone SHALL be read as UTC. Instants SHALL be resolved to millisecond precision. Any other IRI in the `tm:` namespace in these positions SHALL fail with a `Parse` error of dialect SPARQL whose message names the IRI and says that it is not a valid time IRI.

#### Scenario: As of transaction number
- **WHEN** `(v:alice v:worksAt v:acme)` was asserted in tx 3 and retracted in tx 7, and `SELECT ?c FROM <urn:tiramemsu:tm:asOf/5> WHERE { v:alice v:worksAt ?c }` is run on the current view
- **THEN** one row with `c = v:acme` is returned

#### Scenario: As of wall-clock instant
- **WHEN** tx 3 committed at 2026-09-01T10:00:00Z, tx 4 at 2026-09-01T13:00:00Z, and `FROM <urn:tiramemsu:tm:asOf/2026-09-01T12:00:00Z>` is used
- **THEN** the query sees the state as of tx 3

#### Scenario: Instant before the first transaction
- **WHEN** `SELECT * FROM <urn:tiramemsu:tm:asOf/1970-01-02> WHERE { ?s ?p ?o }` is run on a database whose first transaction is later
- **THEN** zero rows are returned and no error is raised

#### Scenario: Timezone offset normalised
- **WHEN** `FROM <urn:tiramemsu:tm:asOf/2026-09-01T14:00:00+02:00>` is used
- **THEN** it selects the same transaction as `FROM <urn:tiramemsu:tm:asOf/2026-09-01T12:00:00Z>`

#### Scenario: Malformed time IRI
- **WHEN** `SELECT * FROM <urn:tiramemsu:tm:asOf/yesterday> WHERE { ?s ?p ?o }` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL naming `urn:tiramemsu:tm:asOf/yesterday`

### Requirement: Default view without time clauses

A query with no time IRI SHALL be evaluated in the view it was submitted on. For a plain database handle that view is the current transaction state with valid time unfiltered. Valid time SHALL never be filtered unless `validAt` is asked for, through the API view or a time IRI.

#### Scenario: Past episodes are visible by default
- **WHEN** `(v:alice v:worksAt v:acme)` is live with valid time `[2020-01-01, 2022-01-01)` and `SELECT ?c WHERE { v:alice v:worksAt ?c }` is run on the current view with no time IRI
- **THEN** one row with `c = v:acme` is returned

### Requirement: FROM sets the query default view

Time IRIs in `FROM` SHALL set the default view of every pattern in the query, including patterns inside `OPTIONAL`, `UNION`, `MINUS`, `EXISTS` and subqueries. The transaction-time part (`asOf`, `history`) and the valid-time part (`validAt`) SHALL be set independently: one `FROM` of each kind may be combined. Each part named by `FROM` SHALL override the same part of the view the query was submitted on. A part that `FROM` does not name SHALL be inherited from that view. Two `FROM` clauses that name the same part with different values SHALL fail with `Unsupported { feature: "conflicting time selectors" }`.

#### Scenario: Combining asOf and validAt
- **WHEN** `SELECT ?c FROM <urn:tiramemsu:tm:asOf/150> FROM <urn:tiramemsu:tm:validAt/2025-03-01> WHERE { v:alice v:worksAt ?c }` is run
- **THEN** only statements live as of tx 150 whose valid interval contains 2025-03-01T00:00:00Z are matched

#### Scenario: FROM overrides the API view's transaction part only
- **WHEN** a query with `FROM <urn:tiramemsu:tm:asOf/100>` is run on the API view as of tx 200 restricted to valid at 2025-01-01
- **THEN** patterns read as of tx 100 and valid at 2025-01-01

#### Scenario: Conflicting selectors rejected
- **WHEN** `SELECT * FROM <urn:tiramemsu:tm:asOf/5> FROM <urn:tiramemsu:tm:history> WHERE { ?s ?p ?o }` is submitted
- **THEN** the request fails with `Unsupported { feature: "conflicting time selectors" }`

#### Scenario: FROM applies inside subqueries and EXISTS
- **WHEN** `SELECT ?p FROM <urn:tiramemsu:tm:asOf/5> WHERE { ?p a v:Person FILTER EXISTS { ?p v:worksAt v:acme } }` is run
- **THEN** both the outer pattern and the `EXISTS` pattern read as of tx 5

### Requirement: GRAPH scopes patterns in time

`GRAPH <time IRI> { … }` SHALL evaluate every pattern inside the block in a view derived from the enclosing scope. The part named by the IRI SHALL be replaced, and the other part SHALL be inherited. `GRAPH` blocks SHALL nest, and the innermost block SHALL win for the part it names. Patterns outside any `GRAPH` block SHALL use the query default. A `GRAPH` time IRI SHALL NOT need to be declared with `FROM NAMED`. A `FROM NAMED` or `USING NAMED` time IRI SHALL be accepted and SHALL have no effect.

#### Scenario: Before and after values of a changed fact
- **WHEN** `(v:alice v:worksAt v:acme)` was live as of tx 150 and was later superseded by `(v:alice v:worksAt v:initech)`, and the query `SELECT ?before ?after WHERE { GRAPH <urn:tiramemsu:tm:asOf/150> { v:alice v:worksAt ?before } v:alice v:worksAt ?after . FILTER(?before != ?after) }` is run on the current view
- **THEN** one row with `before = v:acme` and `after = v:initech` is returned

#### Scenario: Nested scopes combine parts
- **WHEN** `SELECT ?c WHERE { GRAPH <urn:tiramemsu:tm:asOf/150> { GRAPH <urn:tiramemsu:tm:validAt/2021-06-01> { v:alice v:worksAt ?c } } }` is run
- **THEN** the pattern reads statements live as of tx 150 and valid on 2021-06-01

#### Scenario: Inner scope overrides FROM
- **WHEN** `SELECT ?c FROM <urn:tiramemsu:tm:asOf/10> WHERE { GRAPH <urn:tiramemsu:tm:history> { v:alice v:worksAt ?c } }` is run
- **THEN** the pattern reads every statement ever stored, not the state as of tx 10

#### Scenario: History scope for one pattern
- **WHEN** `SELECT ?old WHERE { v:alice v:worksAt ?now . GRAPH <urn:tiramemsu:tm:history> { v:alice v:worksAt ?old } FILTER(?old != ?now) }` is run
- **THEN** every past employer that differs from the current one is returned

### Requirement: Valid-time filtering

A `validAt` selector SHALL keep a statement only when `(v_from is unbounded or v_from ≤ d)` and `(v_to is unbounded or v_to > d)`, where `d` is the selected instant. The interval is half-open. A statement without valid time SHALL be valid at every instant.

#### Scenario: Half-open interval end
- **WHEN** `(v:alice v:worksAt v:acme)` has valid time `[2025-01-01, 2026-03-01)` and `ASK FROM <urn:tiramemsu:tm:validAt/2026-03-01> { v:alice v:worksAt v:acme }` is run
- **THEN** the answer is `false`, and with `validAt/2026-02-28` it is `true`

#### Scenario: Unbounded statements always valid
- **WHEN** `(v:alice v:name "Alice")` has no valid time and `ASK FROM <urn:tiramemsu:tm:validAt/1900-01-01> { v:alice v:name "Alice" }` is run
- **THEN** the answer is `true`

### Requirement: Time IRIs are plain IRIs elsewhere

Outside `FROM`, `FROM NAMED`, `GRAPH`, `USING` and `USING NAMED`, a `tm:` time IRI SHALL be an ordinary IRI with no time meaning. This applies in triple patterns, `FILTER` expressions, `BIND`, `VALUES` and templates.

#### Scenario: Time IRI as data
- **WHEN** `(v:doc v:ref <urn:tiramemsu:tm:asOf/150>)` is inserted and `SELECT ?x WHERE { v:doc v:ref ?x FILTER(?x = <urn:tiramemsu:tm:asOf/150>) }` is run
- **THEN** one row with `x = <urn:tiramemsu:tm:asOf/150>` is returned, and the query is evaluated in the current view

### Requirement: Non-time graphs are unsupported

The store has no named graphs. A `FROM`, `FROM NAMED` or `GRAPH` IRI that is not in the `tm:` namespace SHALL fail with `Unsupported { feature: "named graph" }`, and `GRAPH ?g` SHALL fail with `Unsupported { feature: "GRAPH variable" }`, before execution.

#### Scenario: Ordinary named graph rejected
- **WHEN** `SELECT * FROM <http://example.org/graph1> WHERE { ?s ?p ?o }` is submitted
- **THEN** the request fails with `Unsupported { feature: "named graph" }`

### Requirement: Statement-time virtual predicates

In a triple pattern whose subject is a statement eid, the predicates `tm:txAdded`, `tm:txRetracted`, `tm:validFrom`, `tm:validTo` and `tm:retractKind` SHALL match computed values instead of stored triples. `tm:txAdded` and `tm:txRetracted` bind the adding and retracting transactions, rendered as `urn:tiramemsu:tx:<t>`. `tm:validFrom` and `tm:validTo` bind `xsd:dateTime` values. `tm:retractKind` binds the retraction kind. When the underlying value is absent (a live statement has no `tm:txRetracted`, and an unbounded interval end has no `tm:validFrom` or `tm:validTo`), the pattern SHALL produce no solution for that statement, so `OPTIONAL` is needed to keep it. Transactions bound this way SHALL join with transaction metadata triples such as `sys:author` and `sys:reason`.

#### Scenario: When was a fact added, and by whom
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` was added in tx 42, whose metadata includes `(tx42 sys:author v:agent7)`, and `SELECT ?t ?a WHERE { v:alice v:worksAt v:acme ~ ?r . ?r tm:txAdded ?t . ?t sys:author ?a }` is run
- **THEN** one row with `t = <urn:tiramemsu:tx:42>` and `a = v:agent7` is returned

#### Scenario: Live statement has no retraction
- **WHEN** e1 is live and `SELECT ?r ?x WHERE { v:alice v:worksAt v:acme ~ ?r OPTIONAL { ?r tm:txRetracted ?x } }` is run
- **THEN** one row is returned with `x` unbound

#### Scenario: Retraction time under history
- **WHEN** e1 was retracted in tx 50 and `SELECT ?x FROM <urn:tiramemsu:tm:history> WHERE { v:alice v:worksAt v:acme ~ ?r . ?r tm:txRetracted ?x }` is run
- **THEN** one row with `x = <urn:tiramemsu:tx:50>` is returned

#### Scenario: Valid-from value
- **WHEN** e1 has `v_from` 2020-01-01T00:00:00Z and `SELECT ?f WHERE { v:alice v:worksAt v:acme ~ ?r . ?r tm:validFrom ?f }` is run
- **THEN** one row with `f = "2020-01-01T00:00:00Z"^^xsd:dateTime` is returned
