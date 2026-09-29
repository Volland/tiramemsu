## Purpose

Defines the predicates whose values are computed from a statement's own row (its parts, lifetime, valid time and retraction kind) and from the volatile side table. Queries can then reach statement metadata and high-churn state as ordinary triples, without stored triples or extra joins.

## ADDED Requirements

### Requirement: Recognised virtual predicates
When the predicate position of a TriplePattern is a constant equal to one of the IRIs below, the pattern SHALL be evaluated as a virtual predicate over the statement named by its subject, and SHALL NOT be matched against stored triples with that predicate:

| Predicate | Object value | Kind |
|---|---|---|
| `sys:subject` | the statement's `s` | as stored |
| `sys:predicate` | the statement's `p` | IRI |
| `sys:object` | the statement's `o` | as stored |
| `tm:txAdded` | `t_add` | transaction |
| `tm:txRetracted` | `t_ret` | transaction |
| `tm:validFrom` | `v_from` | datetime (epoch ms, UTC) |
| `tm:validTo` | `v_to` | datetime (epoch ms, UTC) |
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

### Requirement: Virtual predicates resolve to row columns
A virtual-predicate pattern SHALL be computed from the columns of the statement's own row. When its subject is the eid variable of another TriplePattern in the same query that has an identical View, the executor SHALL read the columns of that pattern's row and SHALL NOT add another scan of the triple table. Otherwise it SHALL look the statement up by eid, or scan statements visible in its View when the subject is an unbound variable.

#### Scenario: Statement time on a bound eid costs no extra scan
- **WHEN** an IR joins `TriplePattern(?a, v:worksAt, ?c, eid = ?r)` and `TriplePattern(?r, tm:txAdded, ?t)`, both under Now
- **THEN** the explained SQL contains exactly one alias of the triple table
- **AND** each row binds `?t` to the transaction that added `?r`

#### Scenario: Different views need a lookup
- **WHEN** the eid pattern is under History and the `tm:txRetracted` pattern is under Now
- **THEN** the explained SQL looks up `?r` in a second alias by its integer primary key, with the Now predicate

#### Scenario: Unbound subject scans statements
- **WHEN** an IR `TriplePattern(?r, tm:txAdded, tx 150)` is executed under History
- **THEN** the result binds `?r` to every statement added in transaction 150

### Requirement: Visibility follows the pattern's view
A virtual triple SHALL exist in a View if and only if the statement named by its subject is visible in that View (see view-scoped-scans), and the column it reads is not NULL.

#### Scenario: Retracted statement under Now
- **WHEN** e1 is retracted and `TriplePattern(e1, sys:subject, ?x)` is executed under Now
- **THEN** the result is empty

#### Scenario: Retracted statement under History
- **WHEN** the same pattern is executed under History
- **THEN** the result is one row with `?x = alice`

#### Scenario: Retracted statement under AsOf
- **WHEN** e1 was added at tx 5 and retracted at tx 9, and `TriplePattern(e1, tm:txAdded, ?t)` is executed under `AsOf(Tx(6))`
- **THEN** the result is one row with `?t = tx 5`

### Requirement: Absent values produce no triple
When the column behind a virtual predicate is NULL, the virtual triple SHALL be absent. There SHALL be no row with a missing object.

#### Scenario: Live statement has no txRetracted
- **WHEN** `TriplePattern(?r, tm:txRetracted, ?t)` is executed under Now
- **THEN** the result is empty, because every statement visible under Now is live

#### Scenario: txRetracted and retractKind in history
- **WHEN** e1 was retracted at tx 9 by a cascade, and `Join[TriplePattern(e1, tm:txRetracted, ?t), TriplePattern(e1, tm:retractKind, ?k)]` is executed under History
- **THEN** the result is one row with `?t = tx 9` and `?k = 1`

#### Scenario: Unbounded valid time
- **WHEN** a statement has NULL `v_from` and `v_to = 2026-03-01T00:00:00Z`
- **THEN** `tm:validFrom` produces no triple for it, and `tm:validTo` produces the datetime `2026-03-01T00:00:00Z`

### Requirement: Constant objects compare on the column
When the object of a virtual-predicate pattern is a constant, the executor SHALL compare the underlying column with the constant's value. A constant whose kind cannot be the predicate's object kind (for example an integer for `tm:txAdded`) SHALL make the pattern empty. A constant subject that is not a statement id SHALL make the pattern empty, and a variable subject bound to a non-statement value SHALL match nothing.

#### Scenario: Statements added in one transaction
- **WHEN** an IR `TriplePattern(?r, tm:txAdded, tx 150)` is executed under Now
- **THEN** it returns the live statements added in transaction 150, and the explained plan uses an index on `t_add`

#### Scenario: Wrong object kind
- **WHEN** an IR `TriplePattern(?r, tm:txAdded, 150)` (an integer, not a transaction) is executed
- **THEN** the result is empty

#### Scenario: Non-statement subject
- **WHEN** an IR `TriplePattern(v:alice, sys:subject, ?x)` is executed
- **THEN** the result is empty

#### Scenario: Filter on statement time
- **WHEN** an IR filters `?t > tx 100` over `TriplePattern(?r, tm:txAdded, ?t)`
- **THEN** only statements added after transaction 100 are returned

### Requirement: Virtual patterns do not bind an eid
A virtual-predicate pattern SHALL NOT bind an eid variable, because a virtual triple is not a statement. Such an IR SHALL be rejected with `InvalidQuery`. Under `graph_set = SetOfTriples` and `BagOfEids` alike, a virtual triple SHALL be produced once per visible statement.

#### Scenario: Eid on a virtual pattern
- **WHEN** an IR `TriplePattern(?r, tm:txAdded, ?t, eid = ?x)` is executed
- **THEN** execution fails with `InvalidQuery`

### Requirement: Paths cross layers through statement parts
The `sys:subject` and `sys:object` virtual predicates, and their inverses, SHALL be accepted as steps in PathPattern expressions and passed to the path operator unchanged. The steps themselves are evaluated by the path operator.

#### Scenario: Path expression with a virtual hop
- **WHEN** an IR contains a PathPattern whose path is `v:supportedBy / sys:subject`
- **THEN** planning accepts it and passes the path, including the `sys:subject` step, to the path operator

### Requirement: Volatile keys as virtual properties under Now only
A TriplePattern that opts in to volatile values SHALL, under a View `{Now, Unfiltered}` and with a constant predicate, also match `volatile` rows whose `s` equals the pattern's subject and whose `key` equals the predicate. A `(s, key)` pair that also has a live stored triple with the same subject and predicate SHALL resolve to the stored triple only. Under any other View (AsOf, History, or any `At(d)`), the pattern SHALL match stored triples only, so volatile values are absent. A volatile match has no eid. A pattern that opts in to volatile values and also binds an eid variable SHALL match stored triples only. Patterns that do not opt in SHALL never see volatile values.

#### Scenario: Volatile value under Now
- **WHEN** `volatile` holds `(alice, v:lastSeen, 2026-09-29T10:00Z)`, no triple `(alice lastSeen …)` exists, and an opted-in pattern `TriplePattern(v:alice, v:lastSeen, ?ts)` is executed under Now
- **THEN** the result is one row with `?ts = 2026-09-29T10:00Z`

#### Scenario: Stored triple wins
- **WHEN** alice has both a live triple `(alice lastSeen T1)` and a volatile row `(alice, lastSeen, T2)`
- **THEN** the opted-in pattern returns only `T1`

#### Scenario: Absent under AsOf and History
- **WHEN** the same opted-in pattern is executed under `AsOf(Tx(t))` or under History
- **THEN** it returns no volatile value

#### Scenario: Absent under valid-at
- **WHEN** the opted-in pattern is executed under `{Now, At(d)}`
- **THEN** it returns no volatile value

#### Scenario: Not opted in
- **WHEN** a pattern that does not opt in to volatile values is executed under Now
- **THEN** volatile rows never appear in its result

#### Scenario: Volatile with a variable subject
- **WHEN** an opted-in pattern `TriplePattern(?n, v:lastSeen, ?ts)` is executed under Now
- **THEN** it returns every node with a live `lastSeen` triple, plus every node that has only a volatile `lastSeen` value
