## MODIFIED Requirements

### Requirement: INSERT DATA asserts idempotently

Each ground triple of `INSERT DATA` SHALL be asserted with the idempotent assert operation. Its valid time SHALL be unbounded unless `tm:validFrom` or `tm:validTo` annotations on its reifier set bounds, as the requirement "Valid time writes" describes. A live statement with the same `(s, p, o)` and an overlapping valid time SHALL be reported as existing and SHALL NOT be duplicated. Blank nodes in `INSERT DATA` SHALL become new blank nodes, one per distinct label per request, as SPARQL 1.1 requires. Literal values SHALL be stored in canonical form.

#### Scenario: Repeated insert is idempotent
- **WHEN** `INSERT DATA { v:alice v:worksAt v:acme }` is submitted twice
- **THEN** exactly one live statement `(v:alice v:worksAt v:acme)` exists, and the second report lists its eid as existing

#### Scenario: Blank nodes are fresh per request
- **WHEN** `INSERT DATA { _:b v:name "anon" }` is submitted twice
- **THEN** two distinct blank nodes each have a live `v:name "anon"` statement

#### Scenario: Canonical literal stored
- **WHEN** `INSERT DATA { v:alice v:age "041"^^xsd:integer }` is submitted and `SELECT ?a WHERE { v:alice v:age ?a }` is run
- **THEN** the query returns `"41"^^xsd:integer`

### Requirement: Virtual predicates are read-only

Asserting a triple whose predicate is a transaction-time virtual predicate (`tm:txAdded`, `tm:txRetracted`, `tm:addedAt`, `tm:retractedAt`, `tm:retractKind`) SHALL fail with `Unsupported` naming the predicate before the transaction opens. The same SHALL hold for these predicates in a delete template. Asserting a triple whose predicate is a virtual hop predicate (`sys:subject`, `sys:object`, `sys:predicate`) SHALL fail with `ReservedNamespace` naming the predicate. Triples with the predicate `rdf:reifies` SHALL follow the reifier rules of the RDF 1.2 capability and SHALL never be stored as triples. `tm:validFrom` and `tm:validTo` SHALL follow the requirement "Valid time writes" and SHALL never be stored as triples.

#### Scenario: Valid time cannot be inserted as a triple
- **WHEN** `INSERT { ?r tm:validFrom "2020-01-01T00:00:00Z"^^xsd:dateTime } WHERE { v:alice v:worksAt v:acme ~ ?r }` is submitted
- **THEN** no triple with the predicate `tm:validFrom` is stored, and the matched statement is superseded with `v_from` = 2020-01-01T00:00:00Z

#### Scenario: Transaction time cannot be written
- **WHEN** `INSERT { ?r tm:txAdded 1 } WHERE { v:alice v:worksAt v:acme ~ ?r }` is submitted
- **THEN** the request fails with `Unsupported` naming `tm:txAdded`, and no transaction is created

## ADDED Requirements

### Requirement: Valid time writes

In `INSERT DATA` and in insert templates, a triple with the predicate `tm:validFrom` or `tm:validTo` whose subject denotes a statement SHALL set that bound of the statement's valid time and SHALL NOT be stored as a triple. A subject denotes a statement when it is a statement IRI, a variable bound to a statement, or a reifier (`~`, `{| … |}`, `rdf:reifies`) of a triple in the same request. The object SHALL be an `xsd:dateTime` (its instant), an `xsd:date` (midnight UTC) or an `xsd:integer` (epoch milliseconds).

- A statement that the request asserts through a reifier SHALL be asserted with these bounds. The idempotency of `INSERT DATA` SHALL then use that interval, so an overlapping live statement with the same content is returned unchanged.
- For a statement that was already stored before the operation, all valid-time inserts and deletes of one operation SHALL be merged into a single supersede of that statement with the bounds patched. A patch that leaves the interval unchanged SHALL write nothing.
- A delete template `X tm:validFrom v` or `X tm:validTo v` SHALL clear that bound (unbounded) when `v` equals the current bound, and SHALL otherwise be a no-op. An inserted value SHALL take precedence over a delete of the same bound in the same operation.
- A valid-time triple inside a `GRAPH` block SHALL apply to the statement and SHALL write no membership.
- The update SHALL fail with `InvalidQuery` for:
  - a subject that does not denote a statement;
  - an object of another type;
  - two different inserted values for one bound of one statement in one operation.
- An empty or inverted interval SHALL fail with `InvalidInterval` on assert and with `InvalidPatch` on supersede. In both cases nothing is written.

#### Scenario: Insert a fact with its valid time
- **WHEN** `INSERT DATA { v:alice v:worksAt v:acme {| tm:validFrom "2020-01-01"^^xsd:date ; tm:validTo "2024-01-01T00:00:00Z"^^xsd:dateTime |} }` runs on an empty store
- **THEN** one statement is live with interval [2020-01-01, 2024-01-01), no triple with a `tm:` predicate is stored, and `SELECT ?f ?t WHERE { v:alice v:worksAt v:acme ~ ?e . ?e tm:validFrom ?f ; tm:validTo ?t }` returns those two instants

#### Scenario: Re-inserting an overlapping fact is idempotent
- **WHEN** the same `INSERT DATA` runs again with `tm:validFrom "2021-01-01"^^xsd:date`
- **THEN** no statement is added, the report lists the first eid as existing, and its interval is unchanged

#### Scenario: Close an interval
- **WHEN** `v:alice v:worksAt v:acme` is live from 2020-01-01 with no end, and `INSERT { ?e tm:validTo "2024-01-01"^^xsd:date } WHERE { v:alice v:worksAt v:acme ~ ?e }` runs
- **THEN** the statement is superseded, the new statement has interval [2020-01-01, 2024-01-01), and an annotation of the old statement is on the new one

#### Scenario: Clear a bound
- **WHEN** the statement has interval [2020-01-01, 2024-01-01) and `DELETE { ?e tm:validTo ?t } WHERE { v:alice v:worksAt v:acme ~ ?e . ?e tm:validTo ?t }` runs
- **THEN** the live statement has interval [2020-01-01, open)

#### Scenario: Replace both bounds in one supersede
- **WHEN** `DELETE { ?e tm:validFrom ?f } INSERT { ?e tm:validFrom "2019-06-01"^^xsd:date ; tm:validTo "2023-01-01"^^xsd:date } WHERE { v:alice v:worksAt v:acme ~ ?e . ?e tm:validFrom ?f }` runs
- **THEN** exactly one supersede is reported and the live interval is [2019-06-01, 2023-01-01)

#### Scenario: Epoch milliseconds
- **WHEN** `INSERT DATA { v:a v:p v:b {| tm:validFrom 1000 |} }` runs
- **THEN** the statement's `v_from` is 1000

#### Scenario: Not a statement
- **WHEN** `INSERT DATA { v:alice tm:validFrom "2020-01-01"^^xsd:date }` runs
- **THEN** it fails with `InvalidQuery`, and nothing is written

#### Scenario: Wrong value type
- **WHEN** `INSERT DATA { v:a v:p v:b {| tm:validFrom "soon" |} }` runs
- **THEN** it fails with `InvalidQuery`, and nothing is written

#### Scenario: Inverted interval
- **WHEN** the statement starts on 2020-01-01 and `INSERT { ?e tm:validTo "2019-01-01"^^xsd:date } WHERE { v:alice v:worksAt v:acme ~ ?e }` runs
- **THEN** it fails with `InvalidPatch`, and nothing changes

### Requirement: Statement id and transaction from a triple in SPARQL

The reifier variable of a triple pattern (`~ ?e`) SHALL bind the statement, rendered as `urn:tiramemsu:stmt:<n>`. `?e tm:txAdded ?tx` SHALL bind the transaction that added it, rendered as `urn:tiramemsu:tx:<t>`. Triples with that transaction as subject SHALL be its metadata. `?e tm:txAdded <urn:tiramemsu:tx:<t>>` SHALL match exactly the statements that transaction added.

#### Scenario: Who wrote a fact
- **WHEN** a transaction with `sys:author "agent-7"` and `sys:reason "onboarding"` asserted `v:alice v:worksAt v:acme`, and `SELECT ?e ?tx ?a ?why WHERE { v:alice v:worksAt v:acme ~ ?e . ?e tm:txAdded ?tx . ?tx sys:author ?a ; sys:reason ?why }` runs
- **THEN** one row binds the statement, its transaction, `"agent-7"` and `"onboarding"`

#### Scenario: Everything a transaction wrote
- **WHEN** `SELECT ?e WHERE { ?s ?p ?o ~ ?e . ?e tm:txAdded <urn:tiramemsu:tx:1> }` runs
- **THEN** it returns exactly the statements whose `t_add` is 1, including the transaction's own metadata statements
