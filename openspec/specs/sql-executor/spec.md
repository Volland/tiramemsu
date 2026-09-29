# sql-executor Specification

## Purpose
Defines the small synchronous executor trait through which `tm-core` reaches SQLite: the operations every host must provide, the optional capabilities a host declares, the tier of the engine that runs on the required operations alone, and the `rusqlite` host that ships first.

## Requirements

### Requirement: Required executor operations
`tm-core` SHALL reach SQLite only through a synchronous `Executor` trait that it defines itself, and SHALL NOT depend on `rusqlite` or any other SQLite binding. Every host SHALL provide: prepared statements with bound parameters; interactive transactions (`BEGIN IMMEDIATE` … `COMMIT` / `ROLLBACK`) in which each statement sees the effects of the earlier statements of the same transaction; named savepoints (`SAVEPOINT`, `ROLLBACK TO`, `RELEASE`); and read transactions that observe one stable snapshot until they end.

#### Scenario: The core has no SQLite binding
- **WHEN** the normal (non-dev) dependency tree of `tm-core` is listed
- **THEN** it contains neither `rusqlite` nor `libsqlite3-sys`

#### Scenario: Reads within a write transaction see earlier writes
- **WHEN** one interactive transaction interns a new term and then looks the same value up
- **THEN** the lookup returns the id just inserted, before the transaction commits

#### Scenario: Savepoint rollback keeps the outer transaction
- **WHEN** the engine opens a savepoint inside a write transaction, inserts rows, rolls back to the savepoint and releases it
- **THEN** the inserted rows are gone
- **AND** the outer transaction can still write and commit

#### Scenario: Stable read snapshot
- **WHEN** a read transaction has returned its first row and another connection then commits a write
- **THEN** every later statement of that read transaction still sees the state before the write

### Requirement: Capabilities are declared by the host
Every host SHALL declare a `Capabilities` value with the flags `reader_pool`, `functions` (scalar user functions), `vtab` (virtual tables), `stat4` and `fts5`, and SHALL declare a flag only when the linked SQLite supports it. The engine SHALL read the declaration from the executor instead of probing for features, and SHALL make it available to later crates, so that a component that needs a capability can refuse to start without it. Without `reader_pool`, the writer connection SHALL also serve every read.

#### Scenario: Host without a reader pool
- **WHEN** a database is opened on a host that declares `reader_pool = false`
- **THEN** no reader connection is opened
- **AND** every view reads through the writer connection, serialised with writes by the writer mutex

#### Scenario: Host without STAT4
- **WHEN** a database is opened on a host that declares `stat4 = false`
- **THEN** opening succeeds and planner statistics are kept in `sqlite_stat1` only

#### Scenario: Declaration is visible to later crates
- **WHEN** a later crate asks an open database for its host capabilities
- **THEN** it receives exactly the flags the host declared

### Requirement: The core runs on the required operations alone
Transactions and every write operation (including dry runs and speculation), the temporal views, `View::triples`, the event log, volatile state and the predicate schema SHALL work on a host that provides the required operations and declares no capability. SQL issued by `tm-core` SHALL NOT use user functions, virtual tables or FTS5.

#### Scenario: Core suite on a minimal host
- **WHEN** the `tm-core` integration test suite runs through a host that declares every capability `false`
- **THEN** every scenario of the core capabilities passes, with the same results as on the `rusqlite` host

#### Scenario: No optional feature in core SQL
- **WHEN** every SQL statement that `tm-core` prepares during the test suite is recorded
- **THEN** none of them calls a user function or reads a virtual table

### Requirement: A failed transaction leaves no trace on any host
On every host, a transaction that fails for any reason (an engine error, a caller-aborted body or a host error) SHALL be rolled back through the executor, so that no `tx` row, statement, term, volatile write or counter change remains, and the executor SHALL be usable for the next transaction. Host errors SHALL surface as the host-neutral `Sqlite` error, which carries the SQLite result code.

#### Scenario: Failed body on each host
- **WHEN** a transaction body inserts statements and terms and then fails, once on the `rusqlite` host and once on the minimal host
- **THEN** on both hosts the `meta`, `term`, `tx`, `triple` and `volatile` tables are identical to their state before the transaction
- **AND** the next transaction commits with the next gap-free number

#### Scenario: Host error during a transaction
- **WHEN** the host reports a busy error in the middle of a write transaction
- **THEN** the transaction fails with a `Sqlite` error carrying the busy result code and leaves no trace

### Requirement: The rusqlite host
The crate `tm-rusqlite` SHALL implement the executor on `rusqlite` with bundled SQLite, SHALL declare all five capabilities, and SHALL be the host the `tiramemsu` facade opens by default. Host-specific mechanisms, such as statement caching (`prepare_cached`), the connection handle used inside virtual tables, `rarray`, and the registration of user functions and virtual tables, SHALL stay inside `tm-rusqlite`.

#### Scenario: rusqlite host declares every capability
- **WHEN** a database is opened through the `tiramemsu` facade
- **THEN** the declared capabilities are `reader_pool`, `functions`, `vtab`, `stat4` and `fts5`, all `true`
- **AND** `PRAGMA compile_options` on its connection lists `ENABLE_STAT4` and `ENABLE_FTS5`

#### Scenario: No rusqlite type crosses into the core
- **WHEN** the public items of `tm-core` are inspected
- **THEN** no signature mentions a `rusqlite` type
