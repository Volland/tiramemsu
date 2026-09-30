# sparql-update Specification

## Purpose
Lets applications and agents change the Tiramemsu store with SPARQL 1.1 Update. It defines which update forms exist, how each maps to the store's transaction operations (idempotent assert, retract with cascade), and what the caller gets back.

## Requirements

### Requirement: Updates run as one transaction on the current view

The system SHALL accept SPARQL Update text only on the plain current view: no as-of, history or valid-at selection, and not inside a speculative transaction. Update text on any other view SHALL fail with `Unsupported { feature: "update on a non-current view" }` and change nothing. One update request SHALL be exactly one transaction with one transaction number `t`, including a request of several operations separated by `;`. The operations SHALL run in order, and each SHALL see the effects of the earlier ones. If any operation fails, the whole request SHALL leave no trace: no transaction row, no statements, no terms.

#### Scenario: Multi-operation request is one transaction
- **WHEN** the request `INSERT DATA { v:alice v:worksAt v:acme } ; INSERT DATA { v:bob v:worksAt v:acme }` is submitted on the current view
- **THEN** both statements are live with the same `t_add`, and the report carries that single `t`

#### Scenario: Later operation sees earlier one
- **WHEN** the request `INSERT DATA { v:alice v:age 41 } ; INSERT { ?p v:adult true } WHERE { ?p v:age ?a FILTER(?a >= 18) }` is submitted
- **THEN** `(v:alice v:adult true)` is live after the request

#### Scenario: Failure leaves no trace
- **WHEN** a request inserts `(v:x v:p 1)` and a later operation in the same request violates a `sys:unique` constraint
- **THEN** the request fails with `UniqueViolation`, `(v:x v:p 1)` is not stored, and no transaction number is consumed

#### Scenario: Update on a historical view is rejected
- **WHEN** `INSERT DATA { v:a v:b v:c }` is submitted on the view as of tx 5
- **THEN** the request fails with `Unsupported { feature: "update on a non-current view" }` and nothing is written

### Requirement: Update report

A successful update request SHALL return the transaction report: the transaction number `t`, its instant, the eids newly asserted, the eids that already existed for idempotent inserts, and every retracted eid with its retraction kind (explicit, cascade or cardinality). A request whose operations change nothing SHALL still commit a transaction and return its report.

#### Scenario: Report lists new and existing eids
- **WHEN** `(v:alice v:worksAt v:acme)` is live with eid e1 and `INSERT DATA { v:alice v:worksAt v:acme . v:alice v:age 41 }` is submitted
- **THEN** the report lists e1 under existing, one new eid under asserted, and nothing under retracted

#### Scenario: No-op request still commits
- **WHEN** `DELETE DATA { v:nobody v:knows v:noone }` is submitted and no such triple is live
- **THEN** the request succeeds, a new transaction number is allocated, and the report has empty asserted, existing and retracted lists

### Requirement: INSERT DATA asserts idempotently

Each ground triple of `INSERT DATA` SHALL be asserted with the idempotent assert operation and with an unbounded valid time. A live statement with the same `(s, p, o)` SHALL be reported as existing and SHALL NOT be duplicated. Blank nodes in `INSERT DATA` SHALL become new blank nodes, one per distinct label per request, as SPARQL 1.1 requires. Literal values SHALL be stored in canonical form.

#### Scenario: Repeated insert is idempotent
- **WHEN** `INSERT DATA { v:alice v:worksAt v:acme }` is submitted twice
- **THEN** exactly one live statement `(v:alice v:worksAt v:acme)` exists, and the second report lists its eid as existing

#### Scenario: Blank nodes are fresh per request
- **WHEN** `INSERT DATA { _:b v:name "anon" }` is submitted twice
- **THEN** two distinct blank nodes each have a live `v:name "anon"` statement

#### Scenario: Canonical literal stored
- **WHEN** `INSERT DATA { v:alice v:age "041"^^xsd:integer }` is submitted and `SELECT ?a WHERE { v:alice v:age ?a }` is run
- **THEN** the query returns `"41"^^xsd:integer`

### Requirement: DELETE DATA retracts with cascade

Each ground triple of `DELETE DATA` SHALL retract every live statement with that `(s, p, o)`, whatever its valid time, with retraction kind explicit. Each retraction SHALL cascade to every live statement whose subject or object is a retracted eid, recursively. Deleting a triple that is not live SHALL be a no-op. Retracted statements SHALL stay visible to historical views.

#### Scenario: All episodes retracted
- **WHEN** `(v:alice v:worksAt v:acme)` is live as two episodes with different valid times and `DELETE DATA { v:alice v:worksAt v:acme }` is submitted
- **THEN** both eids are retracted in the same transaction and appear in the report with kind explicit

#### Scenario: Cascade retracts annotations
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` carries the annotation e2 = `(e1 v:confidence 0.8)`, and `DELETE DATA { v:alice v:worksAt v:acme }` is submitted
- **THEN** e1 is retracted with kind explicit, e2 with kind cascade, both with the same `t_ret`, and the view as of the previous transaction still shows both

#### Scenario: Other triples about the same nodes survive
- **WHEN** `(v:alice v:age 41)` is also live and the same `DELETE DATA` is submitted
- **THEN** `(v:alice v:age 41)` stays live

### Requirement: DELETE/INSERT WHERE

For `DELETE { … } INSERT { … } WHERE { … }`, `INSERT { … } WHERE { … }`, `DELETE { … } WHERE { … }` and the short form `DELETE WHERE { … }`, the system SHALL evaluate the `WHERE` pattern once, against the state before the operation. It SHALL then instantiate the delete template for every solution and retract the resulting triples as `DELETE DATA` does. After that it SHALL instantiate the insert template for every solution and assert the resulting triples as `INSERT DATA` does. Template triples with an unbound variable, or with a term that is invalid in its position, SHALL be skipped for that solution. Blank nodes in the insert template SHALL be fresh for each solution. All of this SHALL happen in the request's single transaction.

#### Scenario: Replace a value
- **WHEN** `(v:alice v:worksAt v:acme)` is live and `DELETE { v:alice v:worksAt ?c } INSERT { v:alice v:worksAt v:initech } WHERE { v:alice v:worksAt ?c }` is submitted
- **THEN** the old statement is retracted, `(v:alice v:worksAt v:initech)` is live, and both events carry the same `t`, which the report returns

#### Scenario: WHERE is evaluated before changes
- **WHEN** `(v:a v:next v:b)` and `(v:b v:next v:c)` are live and `DELETE { ?x v:next ?y } INSERT { ?y v:prev ?x } WHERE { ?x v:next ?y }` is submitted
- **THEN** both `v:next` statements are retracted and both `(v:b v:prev v:a)` and `(v:c v:prev v:b)` are live

#### Scenario: Unbound template variable is skipped
- **WHEN** `INSERT { ?p v:ageCopy ?a } WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } }` runs over a person with a name and no age
- **THEN** nothing is asserted for that person and the request succeeds

#### Scenario: DELETE WHERE short form
- **WHEN** `(v:alice v:tag "x")` and `(v:bob v:tag "x")` are live and `DELETE WHERE { ?p v:tag "x" }` is submitted
- **THEN** both statements are retracted in one transaction

#### Scenario: Fresh blank node per solution
- **WHEN** `INSERT { ?p v:card _:c . _:c v:owner ?p } WHERE { ?p a v:Person }` runs over two persons
- **THEN** two distinct blank nodes are created, one per person

### Requirement: Store constraints apply to SPARQL updates

Every assert and retract made by a SPARQL update SHALL go through the same checks as the Rust API: predicate `sys:valueType`, `sys:unique` and `sys:cardinality` schema flags, the reserved `sys:` namespace, the self-reference rule and the cascade limit. A failed check SHALL fail the whole request with the corresponding typed error. Predicate-schema flags, vocab and prefix settings in `sys:` SHALL be insertable through SPARQL.

#### Scenario: Cardinality one replaces the old value
- **WHEN** `v:age` has `sys:cardinality sys:one`, `(v:alice v:age 41)` is live, and `INSERT DATA { v:alice v:age 42 }` is submitted
- **THEN** `(v:alice v:age 42)` is live, and the old statement is retracted with kind cardinality in the same transaction and listed in the report

#### Scenario: Unique violation aborts
- **WHEN** `v:email` has `sys:unique true`, `(v:alice v:email "a@x.org")` is live, and `INSERT DATA { v:bob v:email "a@x.org" }` is submitted
- **THEN** the request fails with `UniqueViolation` and nothing is written

#### Scenario: Schema flag insertable
- **WHEN** `INSERT DATA { v:email sys:unique true }` is submitted on a database whose live data has no duplicate emails
- **THEN** the flag is stored, and later inserts are checked against it

#### Scenario: Reserved namespace rejected
- **WHEN** `INSERT DATA { v:alice sys:supersedes v:bob }` is submitted
- **THEN** the request fails with `ReservedNamespace` naming `urn:tiramemsu:sys:supersedes`

### Requirement: Virtual predicates are read-only

Asserting a triple whose predicate is a statement-time virtual predicate (`tm:txAdded`, `tm:txRetracted`, `tm:validFrom`, `tm:validTo`, `tm:retractKind`) or a virtual hop predicate (`sys:subject`, `sys:object`, `sys:predicate`) SHALL fail with `ReservedNamespace` naming the predicate. Triples with the predicate `rdf:reifies` SHALL follow the reifier rules of the RDF 1.2 capability and SHALL never be stored as triples. Valid time SHALL NOT be settable through SPARQL Update in v1. SPARQL inserts SHALL always have an unbounded valid time.

#### Scenario: Valid time cannot be inserted as a triple
- **WHEN** `INSERT { ?r tm:validFrom "2020-01-01T00:00:00Z"^^xsd:dateTime } WHERE { v:alice v:worksAt v:acme ~ ?r }` is submitted
- **THEN** the request fails with `ReservedNamespace` naming `urn:tiramemsu:tm:validFrom` and nothing is written

### Requirement: Update parse errors report position

Update text that is not valid SPARQL 1.1 Update (with SPARQL 1.2 syntax) SHALL fail with a `Parse` error of dialect SPARQL, carrying the line, column and byte offset of the failure. Nothing SHALL be written. Variables in `INSERT DATA`, and variables or blank nodes in `DELETE DATA` or `DELETE WHERE`, SHALL be reported as parse errors.

#### Scenario: Variable in INSERT DATA
- **WHEN** `INSERT DATA { ?s v:p 1 }` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL and nothing is written

#### Scenario: Blank node in DELETE DATA
- **WHEN** `DELETE DATA { _:b v:p 1 }` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL and nothing is written

### Requirement: Time- and graph-scoped WHERE in updates

The `WHERE` pattern of an update SHALL accept the time IRIs of the temporal dataset capability: in `SERVICE` groups as per-group scopes, and in `USING` as the default for the `WHERE` pattern, with the same meaning that `FROM` has in queries. `USING` and `USING NAMED` with a non-`tm:` IRI, and `GRAPH` with a non-`tm:` IRI or a variable inside the `WHERE` pattern, SHALL name graphs as the `named-graphs` capability specifies. Inside the `WHERE` pattern, `GRAPH` with a time IRI and `SERVICE` with a non-time IRI or a variable SHALL fail exactly as they do in queries. Templates and data blocks SHALL always write to the current state, so a time IRI has no meaning there: a `GRAPH` block in `INSERT DATA`, `DELETE DATA` or a template SHALL name a graph as `named-graphs` specifies, and a time IRI as its name SHALL fail with a `Parse` error that names `SERVICE`. `WITH <g>` SHALL name the graph for template triples outside their own `GRAPH` block and SHALL scope the `WHERE` default graph to `g`. Property paths in the `WHERE` pattern inside `GRAPH` or under a `FROM`-style graph default SHALL fail with `Unsupported { feature: "named graph path" }`.

#### Scenario: Restore a past value
- **WHEN** `(v:alice v:worksAt v:acme)` was live as of tx 150 and has since been retracted, and `INSERT { v:alice v:worksAt ?c } WHERE { SERVICE <urn:tiramemsu:tm:asOf/150> { v:alice v:worksAt ?c } }` is submitted
- **THEN** a new live statement `(v:alice v:worksAt v:acme)` with a new eid is asserted, and the old eid stays retracted

#### Scenario: Time IRI in GRAPH of a WHERE pattern is rejected
- **WHEN** `INSERT { v:alice v:worksAt ?c } WHERE { GRAPH <urn:tiramemsu:tm:asOf/150> { v:alice v:worksAt ?c } }` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL whose message names `SERVICE`, and nothing is written

#### Scenario: Time IRI as the graph of INSERT DATA is rejected
- **WHEN** `INSERT DATA { GRAPH <urn:tiramemsu:tm:asOf/150> { v:a v:b v:c } }` is submitted
- **THEN** the request fails with a `Parse` error of dialect SPARQL whose message names `SERVICE`, and nothing is written, because data is always written to the current state

#### Scenario: GRAPH in INSERT DATA names a graph
- **WHEN** `INSERT DATA { GRAPH <g1> { v:a v:b v:c } }` is submitted
- **THEN** `(v:a v:b v:c)` is live and a member of `g1`

#### Scenario: WITH names a graph
- **WHEN** `WITH <g1> DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }` is submitted over a store where `(v:a v:b v:c)` is in `<g1>` and `(v:d v:e v:f)` is in no graph
- **THEN** the membership of `(v:a v:b v:c)` in `g1` is retracted, the statement stays live, and `(v:d v:e v:f)` is untouched

### Requirement: Unsupported graph operations

The graph-management operations `LOAD`, `ADD`, `MOVE` and `COPY` SHALL fail with `Unsupported` before anything is written, and so SHALL `CLEAR DEFAULT`, `CLEAR ALL`, `DROP DEFAULT` and `DROP ALL`. The feature SHALL be the operation keyword. `CREATE`, `CLEAR GRAPH`, `CLEAR NAMED`, `DROP GRAPH` and `DROP NAMED` are supported, as the `named-graphs` capability specifies. `SILENT` SHALL NOT turn an unsupported operation into success.

#### Scenario: CLEAR DEFAULT is rejected
- **WHEN** `CLEAR DEFAULT` is submitted
- **THEN** the request fails with `Unsupported { feature: "CLEAR DEFAULT" }` and nothing is retracted

#### Scenario: DROP SILENT ALL is still rejected
- **WHEN** `DROP SILENT ALL` is submitted
- **THEN** the request fails with `Unsupported { feature: "DROP ALL" }`

#### Scenario: Rejected operation aborts the whole request
- **WHEN** `INSERT DATA { v:a v:b v:c } ; LOAD <http://example.org/data.ttl>` is submitted
- **THEN** the request fails with `Unsupported { feature: "LOAD" }` and `(v:a v:b v:c)` is not stored
