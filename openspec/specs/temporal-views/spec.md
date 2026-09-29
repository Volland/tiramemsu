# temporal-views Specification

## Purpose
Defines the read side of the core: immutable views that select statements by transaction time (now, as-of a transaction or instant, history) and optionally by valid time, the triple-pattern lookup that runs against a view, the event log read by `since`, and the correctness properties that tie views and the log together.

## Requirements

### Requirement: A view is a transaction-time selector plus a valid-time selector
A view SHALL consist of a transaction-time selector (`Now`, `AsOf(t)` or `History`) and a valid-time selector (`Unfiltered` or `At(ms)`). The default view SHALL be `Now` with `Unfiltered` valid time. A view SHALL be an immutable value: deriving a valid-time view from a view SHALL return a new view and leave the original unchanged. Creating a view SHALL NOT read or write the database.

#### Scenario: Valid time is opt-in
- **WHEN** `(alice worksAt acme)` valid `[2020-01-01, 2022-01-01)` is live and the now view is read without a valid-time selector
- **THEN** the statement is returned, although it is not valid today

#### Scenario: Deriving a view does not change the original
- **WHEN** a valid-at view is derived from a now view
- **THEN** reading the original now view still returns statements regardless of valid time

### Requirement: Now view
A now view SHALL return exactly the statements that are live (`t_ret` absent) in the latest committed state visible to the read's snapshot. Each lookup SHALL run in one read snapshot. Retracted rows SHALL never be returned by a now view.

#### Scenario: Retracted statement disappears
- **WHEN** `e1` is retracted and committed
- **THEN** a subsequent now-view lookup does not return `e1`

#### Scenario: Uncommitted writes are invisible
- **WHEN** a transaction on another thread has inserted statements but not yet committed
- **THEN** a now-view lookup does not return them

### Requirement: As-of view by transaction
An as-of view at transaction `t` SHALL return exactly the statements with `t_add ≤ t` and (`t_ret` absent or `t_ret > t`). As-of at transaction 0 SHALL be empty. In rows returned by an as-of view, a `t_ret` greater than `t` and its `ret_kind` SHALL be reported as absent, so each row shows what was believed at `t`.

#### Scenario: State before a retraction
- **WHEN** `e1` is asserted in transaction 3 and retracted in transaction 6
- **THEN** as-of views at 3, 4 and 5 return `e1` with no `t_ret`
- **AND** as-of views at 2 and 6 do not return `e1`

#### Scenario: Before the first transaction
- **WHEN** an as-of view at transaction 0 is read on a database with committed transactions
- **THEN** it returns no statements

#### Scenario: Future transaction number
- **WHEN** the last committed transaction is 5 and an as-of view at transaction 9 is read
- **THEN** it returns the same statements as the now view at that moment

### Requirement: As-of view by instant
An as-of view at instant `ms` SHALL select the largest committed transaction `t` whose `instant ≤ ms`, resolved within the read's snapshot, and SHALL then behave as the as-of view at `t`. When no committed transaction has `instant ≤ ms`, the view SHALL be empty.

#### Scenario: Instant between transactions
- **WHEN** transaction 1 has instant 1 000 and transaction 2 has instant 2 000, and an as-of view at instant 1 500 is read
- **THEN** it returns the state as of transaction 1

#### Scenario: Instant equal to a transaction instant
- **WHEN** an as-of view at instant 2 000 is read
- **THEN** it returns the state as of transaction 2

#### Scenario: Instant before the first transaction
- **WHEN** an as-of view at instant 999 is read
- **THEN** it returns no statements

#### Scenario: Monotonic resolution with a backwards clock
- **WHEN** transactions commit while the clock moves backwards, so their instants are forced to 10 000, 10 001 and 10 002
- **THEN** as-of views at instants 10 000, 10 001 and 10 002 resolve to transactions 1, 2 and 3 respectively

### Requirement: History view
A history view SHALL return every statement ever inserted by a committed transaction, live or retracted, with its real `t_add`, `t_ret` and `ret_kind`.

#### Scenario: Retracted and live statements together
- **WHEN** `e1` was retracted in transaction 6 and `e2` is live
- **THEN** the history view returns `e1` with `t_ret = 6` and its kind, and `e2` with no `t_ret`

#### Scenario: Asserted and retracted in one transaction
- **WHEN** a statement was asserted and retracted in transaction 7
- **THEN** only the history view returns it, with `t_add = 7` and `t_ret = 7`

### Requirement: Valid-at filter
A valid-at view at instant `d` SHALL keep only statements whose valid interval contains `d`: (`v_from` absent or `v_from ≤ d`) and (`v_to` absent or `v_to > d`). It SHALL combine with any transaction-time selector.

#### Scenario: Half-open boundaries
- **WHEN** a statement is valid `[2025-01-01, 2026-03-01)`
- **THEN** valid-at views at `2025-01-01` and `2026-02-28` return it
- **AND** valid-at views at `2024-12-31` and `2026-03-01` do not

#### Scenario: Unbounded statements are always valid
- **WHEN** a statement has no valid interval
- **THEN** every valid-at view returns it

#### Scenario: Valid-at combined with as-of
- **WHEN** `e1 = (alice worksAt acme)` valid from `2020-01-01` was superseded in transaction 9 with `v_to = 2026-03-01`
- **THEN** the as-of view at transaction 8 filtered at `2026-06-01` returns `e1`
- **AND** the now view filtered at `2026-06-01` returns no employer for alice

### Requirement: Triple-pattern lookup
A view SHALL answer a lookup with an optional subject, predicate and object; absent positions match anything. The result SHALL contain, for every matching statement selected by the view, its eid, subject, predicate, object, `t_add`, `t_ret`, `v_from`, `v_to` and `ret_kind`, ordered by ascending eid. Engine statements (metadata, confirmations, schema flags, supersede links) SHALL be included. A lookup SHALL never write to the database; a constant that needs the dictionary and is not in it SHALL make the result empty.

#### Scenario: Lookup by subject and predicate
- **WHEN** `(alice likes tea)`, `(alice likes coffee)` and `(bob likes tea)` are live and the now view is looked up with subject `alice` and predicate `likes`
- **THEN** exactly the two alice statements are returned, in ascending eid order

#### Scenario: Lookup by object
- **WHEN** the now view is looked up with only object `tea`
- **THEN** both statements whose object is `tea` are returned

#### Scenario: Unknown constant
- **WHEN** a lookup uses a 20-byte string that was never stored
- **THEN** the result is empty and the `term` table is unchanged

#### Scenario: Full scan
- **WHEN** a history view is looked up with no position bound
- **THEN** every statement ever committed is returned

### Requirement: Historical reads are stable
The result of an as-of view at a transaction `t` that was already committed SHALL be identical whenever it is recomputed, whatever transactions (retractions, supersedes, cascades, cardinality replacements) commit later.

#### Scenario: Property test on stability
- **WHEN** for random operation sequences the as-of result at each committed `t` is recorded, and further random transactions then commit
- **THEN** recomputing every recorded as-of result gives exactly the recorded rows

### Requirement: Event log since a transaction
The database SHALL expose the event log as rows `(t, eid, op, kind)`: one `assert` row at `t_add` for every committed statement, with no kind, and one `retract` row at `t_ret` for every retracted statement, with its retraction kind. Events since `t` SHALL return every event with time strictly greater than `t`, ordered by time, then asserts before retracts, then ascending eid. Failed transactions, dry runs and speculative transactions SHALL produce no events.

#### Scenario: Events of a retraction with cascade
- **WHEN** transaction 4 asserts `e1` and `e2 = (e1 :note "x")`, and transaction 5 retracts `e1`
- **THEN** events since 3 are `(4, e1, assert)`, `(4, e2, assert)`, `(5, e1, retract, explicit)` and `(5, e2, retract, cascade)`

#### Scenario: Events since the latest transaction
- **WHEN** events since the last committed transaction are requested
- **THEN** the result is empty

#### Scenario: Assert and retract in one transaction
- **WHEN** transaction 7 asserts and retracts `e5`
- **THEN** events since 6 list `(7, e5, assert)` before `(7, e5, retract, explicit)`

### Requirement: As-of equals replay of the event log
For every committed transaction `t`, the set of statements returned by the as-of view at `t` SHALL equal the set obtained by replaying all events with time `≤ t` in log order, starting from an empty state, where an assert event adds its statement and a retract event removes it.

#### Scenario: Property test over random operations
- **WHEN** random sequences of assert, create, retract, retract-matching, supersede and cardinality-one operations are committed
- **THEN** for every `t` from 0 to the last committed transaction, the as-of view at `t` equals the replayed state at `t`

### Requirement: View scans use covering indexes
The scan predicates generated for each view SHALL let SQLite answer a lookup that projects only statement positions and eids from a covering index: the now view from `live_spo`, `live_pos` or `live_osp`; as-of and history views from `hist_spo`, `hist_pos` or `hist_osp`; and valid-at over the now view from `valid_p` or a `live_*` index. The now-view predicate SHALL contain `t_ret IS NULL` verbatim.

#### Scenario: Query plans
- **WHEN** `EXPLAIN QUERY PLAN` is run for the generated now, as-of, valid-at and object-bound scan shapes
- **THEN** each plan names a covering `live_*`, `hist_*` or `valid_p` index and none performs a full table scan
