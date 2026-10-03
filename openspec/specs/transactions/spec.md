# transactions Specification

## Purpose
Defines the unit of every write: a transaction with a gap-free number and a strictly increasing wall-clock instant, its metadata triples, the report it returns, its options, its all-or-nothing failure behaviour, and the serialisation of all writers through one writer.

## Requirements

### Requirement: Gap-free transaction numbers
Every committed transaction SHALL receive a transaction number `t` equal to the previous committed number plus one, starting at 1 for the first transaction of a database. The number SHALL be allocated inside the writer's SQLite transaction. Failed transactions, dry runs and speculative transactions SHALL NOT consume a number.

#### Scenario: First transaction
- **WHEN** the first transaction of a fresh database commits
- **THEN** its report has `t = 1` and the `tx` table holds exactly one row with `t = 1`

#### Scenario: Numbers stay gap-free across failures and speculation
- **WHEN** transaction 3 commits, then a transaction fails, then a dry run and a speculative transaction run, then another transaction commits
- **THEN** the last transaction has `t = 4`
- **AND** the `tx` table holds exactly the numbers 1 to 4

### Requirement: Strictly increasing transaction instants
Every committed transaction SHALL record an `instant` in epoch milliseconds equal to `max(now_ms, previous_instant + 1)`, where `now_ms` comes from the database's clock and `previous_instant` is the instant of the previous committed transaction (0 for the first). Instants SHALL therefore be strictly increasing in `t`, even when the clock stands still or moves backwards.

#### Scenario: Clock moves backwards
- **WHEN** a transaction commits with the clock at 10 000, and the next commits with the clock set back to 5 000
- **THEN** the first instant is 10 000 and the second is 10 001

#### Scenario: Clock stands still
- **WHEN** three transactions commit while the clock returns the same value 7 000
- **THEN** their instants are 7 000, 7 001 and 7 002

#### Scenario: Normal clock
- **WHEN** the clock returns 20 000 for a transaction whose predecessor has instant 10 001
- **THEN** its instant is 20 000

### Requirement: Every committed transaction is recorded
Every successfully committed transaction SHALL insert exactly one row into the `tx` table, even when all of its operations were idempotent no-ops or it contained no operations at all.

#### Scenario: Empty transaction
- **WHEN** a transaction whose body performs no operation commits
- **THEN** a new `tx` row exists with the next number and a fresh instant
- **AND** its report lists no asserted, existing, retracted or superseded statements

#### Scenario: All operations idempotent
- **WHEN** a transaction only re-asserts statements that are already live with overlapping valid time
- **THEN** a new `tx` row is still recorded
- **AND** the report lists those statements as existing and none as asserted

### Requirement: Transactions are nodes with metadata triples
A transaction SHALL be addressable by a `TX` ObjectId whose payload is `t`. The metadata operation SHALL assert a statement whose subject is the current transaction, whose predicate is `sys:author`, `sys:source`, `sys:reason` or any non-reserved predicate, and whose object is any value. Metadata statements SHALL be ordinary statements with eids, visible in every view that includes their transaction. A `TX` ObjectId MAY also be used as the subject or object of any other statement.

#### Scenario: Author and reason on a transaction
- **WHEN** a transaction records metadata `sys:author = :agent7` and `sys:reason = "user correction"` and commits as `t = 5`
- **THEN** the now view contains the statements `(tx5 sys:author :agent7)` and `(tx5 sys:reason "user correction")`, each with its own eid and `t_add = 5`

#### Scenario: Metadata of a failed transaction disappears
- **WHEN** a transaction records metadata and then fails
- **THEN** no metadata statement from it exists in any view

#### Scenario: Statements about a past transaction
- **WHEN** transaction 8 asserts `(tx5 :reviewedBy :alice)`
- **THEN** the statement is inserted with subject `tx5` and `t_add = 8`

### Requirement: Transaction report
A committed (or dry-run) transaction SHALL return a report containing: `t`; `instant`; `asserted`, the eids of every statement the transaction inserted (including metadata, confirmations, supersede replays and `sys:supersedes` links) in insertion order; `existing`, the eids returned as already-existing by assert operations, without duplicates, excluding eids that the same transaction inserted; `retracted`, every eid whose retraction the transaction recorded, each with its retraction kind, in the order they were retracted; and `superseded`, the `(old, new)` pair for every statement a supersede retracted and replayed, root first.

#### Scenario: Report for mixed operations
- **WHEN** one transaction asserts a new statement A, re-asserts a live statement B, and retracts a statement C that has one annotation D
- **THEN** `asserted = [A]`, `existing = [B]` and `retracted = [(C, explicit), (D, cascade)]`

#### Scenario: Repeated assert inside one transaction
- **WHEN** one transaction asserts the same new statement twice
- **THEN** the first returns new and the second returns existing with the same eid
- **AND** the report lists that eid once in `asserted` and not in `existing`

### Requirement: Transaction options
A transaction SHALL accept options `dry_run` (default false) and `max_cascade` (default 10 000). With `dry_run`, the transaction SHALL execute every operation with full semantics, return the report it would have produced, and then discard all of its effects, recording no `tx` row. `max_cascade` SHALL bound the size of every cascade set computed in the transaction.

#### Scenario: Defaults
- **WHEN** a transaction is run with default options
- **THEN** it commits, and a cascade of 10 000 statements is allowed while one of 10 001 fails

#### Scenario: Dry run returns the would-be report
- **WHEN** a dry run asserts two statements and retracts one
- **THEN** the returned report lists the two eids as asserted and the retraction with its kind
- **AND** afterwards the `tx` and `triple` tables are unchanged
- **AND** the next committed transaction receives the same `t` that the dry-run report showed

### Requirement: Atomic failure leaves no trace
If any operation fails, or the caller's transaction body returns an error, the whole transaction SHALL be rolled back and the error returned to the caller: no `tx` row, no statement, no retraction, no dictionary term, no volatile change and no change to any `meta` counter SHALL remain.

An unwinding transaction callback SHALL undergo the same rollback and dictionary cleanup before its original panic payload is resumed. Catching that unwind SHALL leave the handle usable when rollback succeeds.

#### Scenario: Error after successful operations
- **WHEN** a transaction asserts a statement with a new long string, retracts another statement, and then fails with a unique violation
- **THEN** the error is returned
- **AND** the `tx`, `triple`, `term`, `volatile` and `meta` tables are byte-for-byte equal in content to their state before the transaction

#### Scenario: Caller aborts the body
- **WHEN** the transaction body returns its own error after performing writes
- **THEN** that error is returned to the caller and no effect of the body remains

#### Scenario: Ids from a failed transaction may be reissued
- **WHEN** a transaction that allocated statement id 20 fails, and the next transaction creates a statement
- **THEN** the new statement may receive id 20, because the failed transaction left the counters unchanged

#### Scenario: Transaction callback panics
- **WHEN** the caller catches a Rust unwind after its transaction callback writes facts and dictionary terms and then panics
- **THEN** the original panic payload is resumed after rollback
- **AND** no transaction, fact, term, volatile change, or counter change from the body remains
- **AND** the same handle can commit a subsequent transaction with the next gap-free number

### Requirement: Single writer serialisation
All transactions, dry runs, speculative transactions and schema changes on a database SHALL be executed one at a time through a single writer, in a SQLite write transaction taken with an immediate lock, so that each one observes every effect of the transactions committed before it. Concurrent callers SHALL be serialised rather than rejected.

#### Scenario: Concurrent transactions from many threads
- **WHEN** 8 threads each commit 100 transactions concurrently
- **THEN** exactly 800 transactions commit, numbered 1 to 800 with no gap and no duplicate
- **AND** instants are strictly increasing in transaction number

#### Scenario: Read-modify-write is atomic
- **WHEN** two threads concurrently upsert the same object value on a predicate flagged `sys:unique`
- **THEN** both receive the same node and exactly one live statement holds that value

#### Scenario: Two handles on one file
- **WHEN** two independently opened handles on the same file commit transactions concurrently
- **THEN** SQLite serialises their writes, and transaction numbers across both remain gap-free and unique

### Requirement: Re-entrant writes are rejected
Starting a transaction, dry run or speculative transaction on a database from inside the body or query callback of another transaction or speculative transaction on the same database SHALL fail immediately with a re-entrancy error instead of blocking.

#### Scenario: Nested transaction
- **WHEN** a transaction body calls transact on the same database
- **THEN** the inner call fails with a re-entrancy error
- **AND** the outer transaction can still decide to commit or fail normally
