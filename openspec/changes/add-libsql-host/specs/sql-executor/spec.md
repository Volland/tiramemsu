## MODIFIED Requirements

### Requirement: Required executor operations
`tm-core` SHALL reach SQLite only through a synchronous `Executor` trait that it defines itself, and SHALL NOT depend on `rusqlite`, libSQL or any other SQLite binding. Every host SHALL provide: prepared statements with bound parameters; interactive transactions (`BEGIN IMMEDIATE` … `COMMIT` / `ROLLBACK`) in which each statement sees the effects of the earlier statements of the same transaction; named savepoints (`SAVEPOINT`, `ROLLBACK TO`, `RELEASE`); and read transactions that observe one stable snapshot until they end.

#### Scenario: The core has no SQLite binding
- **WHEN** the normal (non-dev) dependency tree of `tm-core` is listed
- **THEN** it contains none of `rusqlite`, `libsqlite3-sys`, `libsql`, `libsql-ffi`, `libsql-sys` and `libsql-rusqlite`

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
Every host SHALL declare a `Capabilities` value with the flags `reader_pool`, `functions` (scalar user functions), `vtab` (virtual tables), `stat4`, `fts5` and `vector` (libSQL vector columns, `libsql_vector_idx` indexes and `vector_top_k`, used only by an optional approximate tier; exact vector recall requires no capability), and SHALL declare a flag only when the linked SQLite supports it. The engine SHALL read the declaration from the executor instead of probing for features, and SHALL make it available to later crates, so that a component that needs a capability can refuse to start without it. Without `reader_pool`, the writer connection SHALL also serve every read.

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

#### Scenario: Host without vector support
- **WHEN** a database is opened on a host that declares `vector = false`
- **THEN** opening succeeds, no libSQL vector SQL is ever issued on its connections, exact vector recall works, and only libSQL vector operations fail, with `MissingCapability("vector")`

### Requirement: The rusqlite host
The crate `tm-rusqlite` SHALL implement the executor on `rusqlite` with bundled SQLite, SHALL declare `reader_pool`, `functions`, `vtab`, `stat4` and `fts5` and SHALL NOT declare `vector`, and SHALL remain the facade's default native host through the `host-rusqlite` feature. Host-specific mechanisms, such as statement caching (`prepare_cached`), the connection handle used inside virtual tables, `rarray`, and the registration of user functions and virtual tables, SHALL stay inside `tm-rusqlite`.

#### Scenario: rusqlite host declares every capability
- **WHEN** a database is opened through `RusqliteHost`
- **THEN** the declared capabilities are `reader_pool`, `functions`, `vtab`, `stat4` and `fts5`, all `true`, and `vector` is `false`
- **AND** `PRAGMA compile_options` on its connection lists `ENABLE_STAT4` and `ENABLE_FTS5`

#### Scenario: No rusqlite type crosses into the core
- **WHEN** the public items of `tm-core` are inspected
- **THEN** no signature mentions a `rusqlite` type
