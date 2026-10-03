# query-budgets Specification

## Purpose
TBD - created by archiving change add-query-budgets. Update Purpose after archive.

## Requirements

### Requirement: Bounded reader acquisition
Callers SHALL be able to set a reader-acquisition timeout independently of SQLite lock waiting. A timeout SHALL return a typed pool-timeout error without losing reader capacity.

#### Scenario: Pool exhaustion
- **WHEN** all readers are checked out and the acquisition deadline expires
- **THEN** the waiting call returns a pool-timeout error and a later read succeeds after a reader is released

#### Scenario: Available reader
- **WHEN** a reader becomes available before the deadline
- **THEN** the call executes in one committed snapshot

### Requirement: Execution cancellation
A deadline or cancellation request SHALL stop SQL and native execution (native path search SHALL check it during frontier expansion even on a host that cannot interrupt SQL), return a typed cancellation or deadline error, and release read resources. Failed writes SHALL roll back all graph changes.

#### Scenario: Cancel a path
- **WHEN** a native path is cancelled during frontier expansion
- **THEN** execution stops and its reader is reusable

#### Scenario: Cancel a write
- **WHEN** a query write is interrupted before commit
- **THEN** no statement or transaction row from that write is committed

### Requirement: Explicit result budgets
Row and decoded-result-byte budgets SHALL cover the whole operation. Exceeding a budget SHALL fail with a typed limit error; incomplete rows SHALL NOT be presented as a complete result.

#### Scenario: Result overflow
- **WHEN** an operation exceeds its configured result budget
- **THEN** it returns a limit error rather than a successful truncated result

#### Scenario: Composite operation
- **WHEN** a provenance query runs sibling lookups
- **THEN** all lookups consume the same remaining operation budget

### Requirement: Opt-in budgets
Budgets SHALL be opt-in. An operation without a budget, or with an empty one, SHALL behave exactly as before, and the default reader acquisition SHALL wait without limit.

#### Scenario: Empty budget
- **WHEN** a read or write runs with an empty budget
- **THEN** it returns the same result as the same call without a budget

### Requirement: Budgets across the bindings
The JSON bridge SHALL accept the same budget per read, `transact` and `cypherWrite` call, SHALL report each stop with the name of its typed error as the code, and SHALL let another thread cancel a running call by key.

#### Scenario: Cancel by key
- **WHEN** a bridge call runs with a `cancelKey` and another thread calls `cancel` with that key
- **THEN** the call fails with code `Cancelled`
