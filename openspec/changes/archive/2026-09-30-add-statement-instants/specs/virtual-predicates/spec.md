## MODIFIED Requirements

### Requirement: Recognised virtual predicates
When the predicate position of a TriplePattern is a constant equal to one of the IRIs below, the pattern SHALL be evaluated as a virtual predicate over the statement named by its subject, and SHALL NOT be matched against stored triples with that predicate:

| Predicate | Object value | Kind |
|---|---|---|
| `sys:subject` | the statement's `s` | as stored |
| `sys:predicate` | the statement's `p` | IRI |
| `sys:object` | the statement's `o` | as stored |
| `tm:txAdded` | `t_add` | transaction |
| `tm:txRetracted` | `t_ret` | transaction |
| `tm:addedAt` | the `instant` of transaction `t_add` | datetime (the UTC instant, with offset `Z`) |
| `tm:retractedAt` | the `instant` of transaction `t_ret` | datetime (the UTC instant, with offset `Z`) |
| `tm:validFrom` | `v_from` | datetime (the stored UTC instant, with offset `Z`) |
| `tm:validTo` | `v_to` | datetime (the stored UTC instant, with offset `Z`) |
| `tm:retractKind` | `ret_kind` | integer (0 explicit, 1 cascade, 2 supersede, 3 cardinality) |

A pattern whose predicate is a variable SHALL match only stored triples and SHALL NOT produce virtual triples.

#### Scenario: Subject and object of a statement
- **WHEN** e1 = `(alice worksAt acme)` and an IR `TriplePattern(e1, sys:subject, ?x)` is executed under Now
- **THEN** the result is one row with `?x = alice`

#### Scenario: Predicate of a statement
- **WHEN** an IR `TriplePattern(e1, sys:predicate, ?p)` is executed
- **THEN** the result is one row with `?p = v:worksAt`

#### Scenario: Variable predicate does not see virtual triples
- **WHEN** an IR `TriplePattern(e1, ?p, ?o)` is executed and no stored triple has subject e1
- **THEN** the result is empty

#### Scenario: Virtual predicate wins over stored triples
- **WHEN** a stored triple `(e1, tm:txAdded, 999)` exists and e1 was added at tx 5
- **THEN** `TriplePattern(e1, tm:txAdded, ?t)` returns only `?t = tx 5`

## ADDED Requirements

### Requirement: Statement instants come from the transaction table
The object of `tm:addedAt` SHALL be the `instant` of the transaction `t_add` of the statement, and the object of `tm:retractedAt` the `instant` of its `t_ret`, both as a `DATETIME` with offset `Z` encoded like `tm:validFrom`. The executor SHALL compute them from the `tx` table by its integer primary key, with no join added to the query and no extra scan of the triple table when the subject is the eid of another pattern with an identical View. `tm:addedAt` SHALL exist for every visible statement. `tm:retractedAt` SHALL be absent while `t_ret` is NULL. A constant date-time object SHALL compare by instant, whatever its offset, and SHALL match nothing when no transaction has that instant. A constant of any other kind SHALL make the pattern empty. Values SHALL compare with other date-times by instant in filters. Visibility SHALL follow the pattern's View as for every virtual predicate, and the value SHALL be the row's own: under `AsOf(t)`, a statement retracted after `t` is visible and `tm:retractedAt` returns the instant of that later retraction, as `tm:txRetracted` returns its transaction.

#### Scenario: Instant a statement was added
- **WHEN** e1 was added in a transaction committed at `2026-03-10T00:00:00Z` and an IR `TriplePattern(e1, tm:addedAt, ?when)` is executed under Now
- **THEN** the result is one row with `?when = "2026-03-10T00:00:00Z"^^xsd:dateTime`

#### Scenario: Live statement has no retractedAt
- **WHEN** `TriplePattern(?r, tm:retractedAt, ?when)` is executed under Now
- **THEN** the result is empty

#### Scenario: Retraction instant in history
- **WHEN** e1 was retracted in a transaction committed at `2026-03-12T00:00:00Z` and `TriplePattern(e1, tm:retractedAt, ?when)` is executed under History
- **THEN** the result is one row with `?when = "2026-03-12T00:00:00Z"^^xsd:dateTime`

#### Scenario: Later retraction visible under AsOf
- **WHEN** e1 was added at tx 1 and retracted at tx 3, and `TriplePattern(e1, tm:retractedAt, ?when)` is executed under `AsOf(Tx(2))`
- **THEN** the result is one row with the instant of tx 3, as `tm:txRetracted` returns tx 3

#### Scenario: Constant instant with another offset
- **WHEN** the transaction committed at `2026-03-10T00:00:00Z` added two statements and an IR `TriplePattern(?r, tm:addedAt, "2026-03-10T02:00:00+02:00"^^xsd:dateTime)` is executed
- **THEN** the result binds `?r` to those two statements, and the explained plan seeks the `tx_instant` index

#### Scenario: Instant of no transaction
- **WHEN** an IR `TriplePattern(?r, tm:addedAt, "2000-01-01T00:00:00Z"^^xsd:dateTime)` is executed and no transaction committed at that instant
- **THEN** the result is empty

#### Scenario: Wrong object kind for an instant
- **WHEN** an IR `TriplePattern(?r, tm:addedAt, tx 1)` is executed
- **THEN** the result is empty

#### Scenario: Learned late
- **WHEN** e1 is valid from `2026-03-09` and was added at `2026-03-10`, e2 is valid from `2026-04-01` and was added at `2026-03-10`, and SPARQL `SELECT ?r WHERE { ?s ?p ?o ~ ?r . ?r tm:addedAt ?a ; tm:validFrom ?f FILTER(?a > ?f) }` runs
- **THEN** the result is e1 only

#### Scenario: Recorded after it stopped being true
- **WHEN** e3 is valid `[2025-01-01, 2025-06-01)` and was added at `2026-03-10`, e4 is valid `[2025-01-01, 2027-01-01)` and was added at the same instant, and SPARQL `SELECT ?r WHERE { ?s ?p ?o ~ ?r . ?r tm:addedAt ?a ; tm:validTo ?t FILTER(?a > ?t) }` runs
- **THEN** the result is e3 only
