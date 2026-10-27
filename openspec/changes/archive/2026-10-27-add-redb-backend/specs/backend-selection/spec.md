## ADDED Requirements

### Requirement: Detection by file header
`Db::open` SHALL choose the backend from the file's first bytes: `SQLite format 3\0` opens the SQLite backend, the redb magic opens the redb backend, a missing or empty file is created, and any other content fails with `UnknownFormat`.

#### Scenario: Open a redb file without saying so
- **WHEN** a redb database is opened with default options
- **THEN** it opens on the redb backend and `Db::backend()` returns `Backend::Redb`

#### Scenario: Open a SQLite file without saying so
- **WHEN** an existing SQLite database is opened with default options
- **THEN** it opens on the SQLite backend exactly as before this change

#### Scenario: Unknown content
- **WHEN** a file that starts with other bytes is opened
- **THEN** opening fails with `UnknownFormat` and the file is unchanged

### Requirement: Explicit backend choice
`OpenOptions::backend` SHALL accept `Auto`, `Sqlite` and `Redb`. `Auto` creates new files on SQLite; an explicit choice creates new files on that backend, and an explicit choice that disagrees with an existing file SHALL fail with `BackendMismatch` without changing the file.

#### Scenario: New file on redb
- **WHEN** a missing path is opened with `backend: Redb`
- **THEN** a redb database is created there

#### Scenario: Mismatch
- **WHEN** an existing SQLite file is opened with `backend: Redb`
- **THEN** opening fails with `BackendMismatch { expected: Redb, found: Sqlite }`

### Requirement: Creating a database
`Db::create(path, backend, opts)` SHALL create a new database on the given backend and SHALL fail with `AlreadyExists` when the path exists.

#### Scenario: Create refuses an existing file
- **WHEN** `Db::create` is called on an existing path
- **THEN** it fails with `AlreadyExists` and the file is unchanged

### Requirement: Backend compiled out
A build of the facade without the `redb` feature SHALL fail to open or create a redb database with `BackendNotCompiled { backend: Redb }`.

#### Scenario: Default Rust build opens a redb file
- **WHEN** a facade built without the `redb` feature opens a redb file
- **THEN** opening fails with `BackendNotCompiled`
