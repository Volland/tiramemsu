# speculative-transactions Specification

## Purpose
Defines speculative transactions: applying a transaction hypothetically with the full engine, querying the resulting uncommitted state, and discarding it so that nothing reaches history, while guaranteeing that no id shown during speculation is ever issued again.

## Requirements

### Requirement: Speculation runs the full engine and exposes uncommitted state
A speculative transaction SHALL take the single writer, open a savepoint, run the caller's operations with full semantics (schema checks, cascade, supersede, cardinality-one, upsert, metadata, volatile), and then call the caller's query callback with a now view that reads on the writer connection, so that it sees the uncommitted state, optionally narrowed by a valid-at filter. The value returned by the callback SHALL be returned to the caller.

#### Scenario: Hypothetical assert is visible inside
- **WHEN** a speculative transaction asserts `(alice worksAt globex)` and its callback looks up alice's employer
- **THEN** the callback sees `globex`, and the value it returns is returned to the caller

#### Scenario: Hypothetical retraction with cascade
- **WHEN** a speculative transaction retracts `e1` that has annotation `e2`
- **THEN** inside the callback neither `e1` nor `e2` is in the now view

#### Scenario: Other readers never see speculation
- **WHEN** a reader on another thread looks up the same pattern while the callback is running
- **THEN** it sees only committed state

### Requirement: Speculation leaves no trace
After a speculative transaction finishes, whether its operations or callback succeeded or failed, the savepoint SHALL be rolled back and released. The `triple`, `term`, `tx` and `volatile` tables SHALL be unchanged, no transaction number SHALL be consumed, and no event SHALL appear in the log.

Unwinding callbacks SHALL receive the same cleanup, including dictionary rollback and burning allocated ids, before the original panic payload is resumed.

#### Scenario: Tables unchanged after speculation
- **WHEN** a speculative transaction asserts statements with new long strings, supersedes a statement and sets a volatile value
- **THEN** afterwards the `triple`, `term`, `tx` and `volatile` tables have exactly their previous contents

#### Scenario: Operations fail
- **WHEN** the speculative operations fail with `UniqueViolation`
- **THEN** that error is returned, the callback is not called, and the tables are unchanged

#### Scenario: Callback fails
- **WHEN** the callback returns an error
- **THEN** that error is returned and the tables are unchanged

#### Scenario: Transaction numbers are not consumed
- **WHEN** the last committed transaction is 4, a speculative transaction runs, and then a transaction commits
- **THEN** the committed transaction has number 5

### Requirement: Ids allocated during speculation are burned
After the savepoint is rolled back, the writer SHALL re-apply the advanced `next_stmt`, `next_node`, `next_bnode` and `next_term` counters in a small commit of their own, so that no statement id, node id, blank-node id or term id allocated during speculation (or during a dry run) is ever issued again. `last_t` and `last_instant` SHALL NOT be advanced. When speculation allocated no id, no counter commit SHALL be made.

#### Scenario: Statement ids are not reissued
- **WHEN** a speculative transaction creates statements with eids up to `e20`, and a transaction then commits a new statement
- **THEN** the new statement's eid is greater than `e20`

#### Scenario: Node ids are not reissued
- **WHEN** a speculative transaction creates node `n7` and a later transaction creates a node
- **THEN** the later node is not `n7`

#### Scenario: Term ids are not reissued
- **WHEN** a speculative transaction interns a new IRI with term id 30, and a later transaction interns the same IRI
- **THEN** the IRI receives a new term id greater than 30

#### Scenario: Dry run burns ids the same way
- **WHEN** a dry run creates statements and then a transaction commits
- **THEN** only the id counters in `meta` changed as a result of the dry run, and the committed statements have larger eids than any the dry-run report listed

#### Scenario: Burned ids survive a reopen
- **WHEN** a speculative transaction burns ids, the database is closed and reopened, and a statement is created
- **THEN** its eid is larger than every eid burned before the reopen

#### Scenario: Speculative operations or query panic
- **WHEN** a dry-run callback, speculative operations callback, or speculative query callback panics after allocating ids
- **THEN** the savepoint is rolled back and released, the dictionary cache is restored, and allocated node, blank-node, statement, and term ids are burned
- **AND** no graph history or event is committed and no transaction number is consumed
- **AND** the original panic payload is resumed after cleanup and subsequent operations can use the same handle

### Requirement: Speculation holds the single writer and starts from now
A speculative transaction SHALL hold the writer for its whole duration, so committed transactions from other callers wait until it finishes. Speculation SHALL always start from the latest committed state; there is no way to speculate from a past transaction and no persistent branch.

#### Scenario: Writers wait for speculation
- **WHEN** a transaction is started on another thread while a speculative callback is running
- **THEN** that transaction commits only after the speculation has finished, and it does not see any speculative write

#### Scenario: Nested write from the callback
- **WHEN** the speculative callback starts a transaction on the same database
- **THEN** that call fails with a re-entrancy error
