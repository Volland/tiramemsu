## ADDED Requirements

### Requirement: libSQL executor host
The crate `tm-libsql` SHALL implement `Host` and `Executor` on libSQL through a synchronous binding with bundled C sources, providing every required executor operation and the same registration hooks as `tm-rusqlite`: scalar and aggregate functions, eponymous table functions, connection-aware table functions that read through the calling statement's connection, the `rarray` table function, and interruption of running statements through `Executor::set_interrupt`. Host-specific mechanisms SHALL stay inside `tm-libsql`, and no libSQL type SHALL appear in a public signature of `tm-core`.

#### Scenario: Engine runs on libSQL
- **WHEN** a database is opened with `LibsqlHost` and the query engine enabled
- **THEN** transactions, views, SPARQL, Cypher, paths, LFTJ and text recall return the same results as on `RusqliteHost` for the same operations

#### Scenario: Interrupted statement
- **WHEN** a budgeted query whose deadline expires runs on `LibsqlHost`
- **THEN** the running statement stops and the operation fails with `DeadlineExceeded`, and the reader returns to the pool without an installed interrupt

#### Scenario: Typed error from a table function
- **WHEN** a path query on `LibsqlHost` exceeds `path_max_states` inside `tm_path`
- **THEN** the executor call fails with `PathLimitExceeded`, not a generic SQLite error

### Requirement: Probed capabilities including vector
`LibsqlHost` SHALL establish each capability by running it on a scratch connection: `functions`, `vtab`, `fts5` and `vector` by exercising them, `stat4` from `PRAGMA compile_options`, and `reader_pool` as true for file storage. `vector` SHALL be declared only if a table with an `F32_BLOB` column, a `libsql_vector_idx` index on it and a `vector_top_k` query over it all work. The host SHALL report the SQLite version, the libSQL version when available, the compile options and the probed set.

#### Scenario: Full capability set
- **WHEN** `LibsqlHost::new()` is constructed on a supported native target
- **THEN** it declares `reader_pool`, `functions`, `vtab`, `stat4`, `fts5` and `vector`, all `true`

#### Scenario: Narrowed capabilities
- **WHEN** a test narrows the host's capabilities to exclude `vector`
- **THEN** every libSQL vector operation fails with `MissingCapability("vector")` and every other operation, including exact vector recall, still works

### Requirement: One native host per binary
The facade SHALL select its native host through the cargo features `host-rusqlite` and `host-libsql`. Exactly one native host SHALL be linked into a binary; enabling both SHALL fail at compile time with a message naming the conflict, and enabling neither SHALL leave `Db::open_with_host` as the only way to open a database. `Db::open` SHALL use the selected host.

#### Scenario: Both hosts selected
- **WHEN** the facade is built with both `host-rusqlite` and `host-libsql`
- **THEN** compilation fails with an error that names the two features

#### Scenario: Host reported by an open database
- **WHEN** a database is opened through `Db::open` in a build with `host-libsql`
- **THEN** its capabilities include `vector = true`

### Requirement: Every suite runs on both native hosts
Once `tm-libsql` exists, continuous integration SHALL run the full workspace test suite, the SPARQL and Cypher differential suites, the TCK harness, the facade feature matrix and the Python and Node smoke tests once per native host on Linux, macOS and Windows, and SHALL fail on any difference in results or conformance counts between the hosts.

#### Scenario: Matrix difference
- **WHEN** a query returns different rows on the libSQL build than on the rusqlite build
- **THEN** the host matrix job fails

### Requirement: File interchange between hosts
A tiramemsu file that holds no vector index SHALL open under either native host after being written by the other, with every `meta`, `term`, `tx`, `triple`, `volatile`, `pred_multi` and saved-answer row unchanged, and SHALL continue transaction numbering and id allocation. Neither host SHALL enable libSQL-specific file features (virtual WAL, random rowid, encryption).

#### Scenario: Stock SQLite to libSQL and back
- **WHEN** a file is written by the rusqlite build, closed, opened and extended by the libSQL build, closed, and opened by the rusqlite build again
- **THEN** every row written by either build is present and unchanged and the next transaction number follows the last one

#### Scenario: Uncleanly closed writer
- **WHEN** a writer process of one build is killed after a commit and the file is opened by the other build
- **THEN** the committed transaction is visible and no uncommitted one is

### Requirement: Stock-SQLite hosts open files with vector indexes
If a future approximate tier creates libSQL vector indexes, a host that does not declare `vector` SHALL open such a file without error, SHALL NOT read, write or rebuild any `vec_` table, SHALL NOT issue any statement that evaluates a libSQL index expression (schema-wide `ANALYZE`, `PRAGMA integrity_check`, `VACUUM`, `REINDEX`), and SHALL report requests for the approximate tier as `MissingCapability("vector")` while exact vector recall keeps working.

#### Scenario: Ordinary work on a stock host
- **WHEN** a file with a vector index is opened by the rusqlite build and statements are asserted, retracted, queried and text-recalled
- **THEN** every operation succeeds and the vector tables are byte-for-byte unchanged

#### Scenario: Approximate tier requested on a stock host
- **WHEN** the libSQL approximate tier is requested on that host
- **THEN** it fails with `MissingCapability("vector")`, and an exact vector search on the same host succeeds

### Requirement: libSQL stays an opt-in native host
`host-rusqlite` SHALL remain the default native host of the `tiramemsu` facade, the JSON bridge, the MCP server and the Python and Node bindings; `host-libsql` SHALL be opt-in only. Vector recall SHALL NOT depend on the libSQL host or on the `vector` capability.

#### Scenario: Default build
- **WHEN** an application depends on `tiramemsu` with default features
- **THEN** it links the rusqlite host and vector recall works without `vector`

#### Scenario: Opting into libSQL
- **WHEN** an application builds the facade with `default-features = false` and `host-libsql`
- **THEN** it opens the same files and every operation, including exact vector recall, returns the same results as on the rusqlite host
