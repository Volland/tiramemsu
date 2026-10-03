# wasm-sqlite-host Specification

## Purpose
TBD - created by archiving change add-wasm-sqlite-host. Update Purpose after archive.

## Requirements

### Requirement: Honest runtime capabilities
The WASM host SHALL report actual supported capabilities and SHALL reject unsupported query-engine initialization without degrading query semantics.

#### Scenario: Core-only runtime
- **WHEN** the runtime lacks virtual-table registration
- **THEN** core operations work and enabling the query engine returns MissingCapability

#### Scenario: Native operator support
- **WHEN** query execution is enabled
- **THEN** every required scalar function and native table function is registered on each execution connection

### Requirement: Durable format interoperability
Supported durable storage SHALL preserve the SQLite format, transaction invariants, and journal contract. Unsupported persistence configurations SHALL fail explicitly.

#### Scenario: Native interchange
- **WHEN** a file is exported after WASM commits and opened natively
- **THEN** terms, statements, transaction history, and allocated ids are preserved

#### Scenario: Unsupported journal
- **WHEN** a selected storage runtime cannot satisfy the required journal mode
- **THEN** opening reports the limitation rather than claiming durable compatible operation

### Requirement: Worker lifecycle recovery
Worker interruption SHALL leave only committed transactions in durable storage. Reopening SHALL resume counters and views from the persisted database.

#### Scenario: Interrupted write
- **WHEN** a worker is stopped during an uncommitted write
- **THEN** reopening recovers the last committed state

#### Scenario: Restart
- **WHEN** a worker restarts after a successful commit
- **THEN** the fact is visible and the next transaction number advances normally

### Requirement: Explicit storage availability
Each storage SHALL be opened only where its VFS exists; a storage unavailable on the running target or not installed SHALL fail at open with a typed error.

#### Scenario: Storage unavailable
- **WHEN** OPFS storage is opened outside a dedicated worker or before its VFS is installed, or a browser storage is opened on a native target
- **THEN** opening fails with `MissingCapability` or `Unsupported` and no database is created

#### Scenario: Native file reaches the browser
- **WHEN** a database closed by the native host is imported into browser storage
- **THEN** the imported main file alone holds every committed transaction and the WASM host continues its numbering

### Requirement: Worker binding refuses unavailable calls
The WebAssembly binding SHALL refuse calls that cannot run on the target instead of failing at runtime.

#### Scenario: Time-dependent call
- **WHEN** a call carries a budget, or is a bulk import operation or `cancel`
- **THEN** it fails with `Unsupported` and the database is unchanged
