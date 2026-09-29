# volatile-state Specification

## Purpose
Defines the volatile side table for high-churn state (last-seen times, counters, per-turn scores) that does not deserve history: values are plain upserts outside the graph, visible only in the now view, and never mistaken for statements.

## Requirements

### Requirement: Volatile values are upserted per subject and key
Setting a volatile value SHALL upsert the row `(s, key)` with the encoded value and `updated_at` equal to the current transaction's instant, replacing any previous value for that pair. The key SHALL be an `IRI` and the subject any subject-capable ObjectId; other kinds SHALL fail with an invalid-term error. Clearing a volatile value SHALL remove the row for `(s, key)` and SHALL succeed when no row exists. Volatile writes SHALL be part of their transaction: rolled back when it fails, discarded by dry runs and speculation.

#### Scenario: Overwrite a counter
- **WHEN** transaction 3 sets `(alice, :lastSeen)` to `2026-09-01T10:00Z`, and transaction 4 sets it to `2026-09-02T10:00Z`
- **THEN** the volatile table holds one row for `(alice, :lastSeen)` with the second value and `updated_at` equal to transaction 4's instant

#### Scenario: Failed transaction discards the volatile write
- **WHEN** a transaction sets a volatile value and then fails
- **THEN** the volatile table is unchanged

#### Scenario: Clear a value
- **WHEN** a volatile value for `(alice, :score)` is cleared, and then cleared again
- **THEN** the row is gone and both calls succeed

### Requirement: Volatile values are not statements
Volatile values SHALL have no eid, no transaction-time lifetime, no valid time and no layers. They SHALL NOT appear in any triple lookup, in the history view or in the event log, and setting one SHALL NOT insert or retract any statement. A transaction whose only operation is a volatile write SHALL still record a `tx` row.

#### Scenario: Not visible as a triple
- **WHEN** `(alice, :lastSeen)` is set as a volatile value
- **THEN** a now-view triple lookup with subject `alice` and predicate `:lastSeen` returns nothing
- **AND** events since the previous transaction contain no row for it

### Requirement: Volatile values are visible only in the now view
A view SHALL resolve the values of a subject and key as follows: if statements `(s, key, o)` are selected by the view, their objects SHALL be returned (a key present both as a statement and as a volatile value resolves to the statement); otherwise, only for a now view, the volatile value SHALL be returned if one exists. As-of and history views SHALL return no volatile value, so volatile properties are absent rather than wrong under time travel.

#### Scenario: Now view reads the volatile value
- **WHEN** `(alice, :lastSeen)` is volatile and no statement `(alice :lastSeen ?)` is live
- **THEN** resolving `alice`'s `:lastSeen` in the now view returns the volatile value

#### Scenario: Absent under time travel
- **WHEN** the same key is resolved in an as-of view at the latest transaction, and in the history view
- **THEN** both return no value

#### Scenario: Statement wins over volatile
- **WHEN** `(alice :status "away")` is a live statement and `(alice, :status)` is also volatile with `"busy"`
- **THEN** resolving `alice`'s `:status` in the now view returns `"away"`

#### Scenario: Speculation sees its own volatile writes
- **WHEN** a speculative transaction sets `(alice, :score)` to 5 and its callback resolves it
- **THEN** the callback sees 5, and after speculation the committed value is unchanged
