# storage-format Specification

## Purpose
Defines how a Tiramemsu database is laid out in a single SQLite file: creation, opening, journal mode, the STRICT format-version-1 schema and the names it reserves, the engine metadata counters, the planner statistics kept with the store, and how format versions are checked and migrated forward.

## Requirements

### Requirement: Opening creates a new database file
Opening a path where no file exists SHALL create a new SQLite file and initialise the complete format-version-1 schema (tables `meta`, `term`, `tx`, `triple`, `volatile`, every index, the `event` view and every invariant trigger) inside one SQLite transaction before the database is returned to the caller.

#### Scenario: Fresh file gets the full schema
- **WHEN** a database is opened at a path that does not exist
- **THEN** the file is created
- **AND** it contains the tables `meta`, `term`, `tx`, `triple` and `volatile`, the indexes `term_key`, `term_num`, `tx_instant`, `live_spo`, `live_pos`, `live_osp`, `hist_spo`, `hist_pos`, `hist_osp`, `valid_p`, `log_add` and `log_ret`, the view `event`, and the triggers `triple_no_delete`, `triple_retract_once`, `term_no_delete`, `term_no_update`, `tx_no_delete` and `tx_no_update`

#### Scenario: Fresh database is empty
- **WHEN** a database is freshly created
- **THEN** the `tx`, `triple`, `term` and `volatile` tables contain no rows
- **AND** a now view, a history view and an as-of view at any point in time all return no statements

#### Scenario: Interrupted creation leaves no half-initialised file
- **WHEN** schema initialisation fails part-way (for example the disk is full)
- **THEN** opening returns an error
- **AND** the file contains no tiramemsu tables, so a later open retries initialisation from scratch

### Requirement: Opening an existing database preserves its contents
Opening a path that holds a format-version-1 Tiramemsu database SHALL NOT modify any existing row of `term`, `tx` or `triple`, and SHALL resume id allocation and transaction numbering from the values recorded in `meta`.

#### Scenario: Reopen continues numbering
- **WHEN** a database with last committed transaction 7 is closed and reopened, and a new transaction commits
- **THEN** the new transaction has number 8
- **AND** every statement id and node id it allocates is greater than every id allocated before the reopen

#### Scenario: Reopen keeps history
- **WHEN** a database is reopened
- **THEN** the history view returns exactly the same statements, with the same lifetimes and valid times, as before it was closed

### Requirement: Tables are STRICT and the journal is WAL
Every tiramemsu table SHALL be declared STRICT, and every connection the engine opens SHALL use `journal_mode = WAL` and `synchronous = NORMAL`.

#### Scenario: Journal mode is WAL
- **WHEN** a database is opened and `PRAGMA journal_mode` is queried on any engine connection
- **THEN** the result is `wal`

#### Scenario: STRICT rejects wrongly typed values
- **WHEN** a raw SQLite connection inserts a TEXT value into the `s` column of `triple`
- **THEN** SQLite rejects the insert with a datatype error

### Requirement: Schema matches format version 1 exactly
The tables, columns, column types, nullability, primary keys, indexes (including their column order, sort direction and partial `WHERE` clauses), the `event` view and the triggers created for format version 1 SHALL be exactly those of the documented format-1 DDL, so that files are interchangeable between builds. SQLite's own statistics tables (`sqlite_stat1`, `sqlite_stat4`) are not part of the format. The `live_*` indexes SHALL keep `t_ret` in their key although it is always NULL there, because without it SQLite 3.53 no longer treats them as covering for `t_ret IS NULL` and picks a `hist_*` index instead.

#### Scenario: Partial live indexes
- **WHEN** the schema of a fresh database is inspected
- **THEN** `live_spo`, `live_pos`, `live_osp` and `valid_p` are partial indexes with the condition `t_ret IS NULL`
- **AND** `log_ret` is a partial index with the condition `t_ret IS NOT NULL`
- **AND** `term_num` is a partial index with the condition `num IS NOT NULL`

#### Scenario: Live indexes keep t_ret in their key
- **WHEN** the columns of `live_spo`, `live_pos` and `live_osp` are inspected
- **THEN** each has its three position columns followed by `t_ret`, `v_from` and `v_to`
- **AND** `EXPLAIN QUERY PLAN` for the "now" shape `s=? AND p=? AND t_ret IS NULL` names `COVERING INDEX live_spo`

#### Scenario: History indexes order newest first
- **WHEN** the schema of a fresh database is inspected
- **THEN** `hist_spo`, `hist_pos` and `hist_osp` contain `t_add` in descending order after their three position columns

#### Scenario: Volatile table has a composite key without rowid
- **WHEN** the schema of a fresh database is inspected
- **THEN** `volatile` is a WITHOUT ROWID STRICT table whose primary key is `(s, key)`

### Requirement: Format 1 reserves names for later milestones
Format 1 SHALL reserve, and SHALL NOT create, the table `seal_key` (crypto-shredding, milestone M6), the table `term_fts` and every name starting with `vec_` (retrieval, milestone M7), alongside ObjectId tag 15 `SEALED` and the schema flag `sys:sensitive`. Only the migrations of those milestones MAY create objects with these names.

#### Scenario: Reserved names are absent
- **WHEN** the schema of a fresh or reopened format-1 database is inspected
- **THEN** it contains no table, index, view or trigger named `seal_key` or `term_fts`, and none whose name starts with `vec_`

### Requirement: Planner statistics are part of the store
Planner statistics SHALL be kept current by the engine, not left to the caller. Every open SHALL run `PRAGMA optimize=0x10002` after initialisation or migration, which analyses tables that were never analysed. The writer SHALL run `PRAGMA optimize` after a bulk load and at most once every `OpenOptions.optimize_every` commits (default 1000). `Db::optimize()` SHALL run a full `ANALYZE`. Statistics SHALL live only in SQLite's own `sqlite_stat1` (and `sqlite_stat4` when the host declares `stat4`), outside the graph and outside never-forget, and SHALL only ever change a plan's speed, never a result.

#### Scenario: Statistics exist without an explicit optimize
- **WHEN** a database is opened, loaded with a skewed data set through the ordinary write API, closed and reopened, without any call to `Db::optimize()`
- **THEN** `sqlite_stat1` holds rows for the `triple` indexes

#### Scenario: Periodic optimize
- **WHEN** a database is opened with `optimize_every = 10` and 25 small transactions commit
- **THEN** the writer has run `PRAGMA optimize` exactly twice after opening

#### Scenario: Stale statistics never change results
- **WHEN** heavy churn is committed without re-analysis, a set of lookups over every view runs, then `Db::optimize()` runs and the same lookups run again
- **THEN** both runs return identical results

#### Scenario: Full analysis on request
- **WHEN** `Db::optimize()` is called
- **THEN** it runs `ANALYZE` and `sqlite_stat1` holds a row for every index of `triple`

### Requirement: Multi-eid predicates are recorded
The writer SHALL add a predicate to `pred_multi`, within the same transaction, whenever it inserts a statement while another row (live or retracted) with the same `(s, p, o)` exists. It SHALL never remove a predicate from `pred_multi`, and SHALL increase `meta.multi_version` whenever it adds one. A rolled-back transaction, a speculation or a dry run SHALL leave `pred_multi` unchanged.

#### Scenario: Idempotent assert records nothing
- **WHEN** `(v:alice v:name "Alice")` is asserted twice with overlapping valid time
- **THEN** `pred_multi` does not contain `v:name`

#### Scenario: Parallel edge and episode record the predicate
- **WHEN** `create` makes a second `(v:alice v:called v:bob)`, and `(v:alice v:worksAt v:acme)` is asserted again for a non-overlapping valid interval
- **THEN** `pred_multi` contains `v:called` and `v:worksAt`, and `meta.multi_version` has increased by 2

#### Scenario: Re-assert after retract records the predicate
- **WHEN** `(v:alice v:likes v:tea)` is asserted, retracted, and asserted again
- **THEN** `pred_multi` contains `v:likes`

### Requirement: Engine metadata counters
The `meta` table SHALL hold exactly the integer keys `format_version`, `next_term`, `next_node`, `next_bnode`, `next_stmt`, `last_t`, `last_instant` and `multi_version`. A fresh database SHALL start with `format_version = 1`, `last_t = 0`, `last_instant = 0`, `multi_version = 0`, and every `next_*` counter at 1. Every id the engine allocates SHALL come from the corresponding counter and never from the maximum existing rowid, and each counter SHALL only ever increase.

#### Scenario: Fresh counters
- **WHEN** a database is freshly created
- **THEN** `meta` contains `format_version = 1`, `next_term = 1`, `next_node = 1`, `next_bnode = 1`, `next_stmt = 1`, `last_t = 0`, `last_instant = 0` and `multi_version = 0`

#### Scenario: Counters advance with allocation
- **WHEN** a committed transaction creates 3 statements and 1 new node
- **THEN** `next_stmt` has advanced by at least 3, `next_node` by at least 1, and `last_t` and `last_instant` equal the new transaction's number and instant

#### Scenario: Ids are not derived from existing rows
- **WHEN** `next_stmt` is ahead of the largest stored statement id (because earlier ids were burned by a speculative transaction)
- **THEN** the next statement id allocated equals `next_stmt`, not the largest stored id plus one

### Requirement: Newer format versions are refused
Opening a file whose `meta.format_version` is greater than the highest version this build supports SHALL fail with a `FormatVersion { found, supported }` error and SHALL NOT modify the file.

#### Scenario: File from a newer build
- **WHEN** a file with `format_version = 2` is opened by a build that supports only version 1
- **THEN** opening fails with `FormatVersion { found: 2, supported: 1 }`
- **AND** the file's bytes are unchanged

### Requirement: Older format versions are migrated forward atomically
Opening a file whose `meta.format_version` is lower than the supported version SHALL apply every migration from the found version to the supported version, in order, inside one SQLite transaction, and SHALL set `format_version` to the supported version at the end of that transaction. A migration SHALL only add tables, columns, indexes, views or triggers, and SHALL NOT delete or rewrite any row of `triple`, `term` or `tx`.

#### Scenario: Successful migration
- **WHEN** a file at an older supported version is opened
- **THEN** all pending migrations are applied in version order
- **AND** `format_version` equals the supported version
- **AND** every pre-existing triple, term and transaction row is unchanged

#### Scenario: Failed migration rolls back
- **WHEN** one migration step fails while opening an older file
- **THEN** opening returns an error
- **AND** the file is left at its original format version with its original contents

### Requirement: Foreign files are refused
Opening an existing SQLite file that contains user tables but no tiramemsu `meta` table SHALL fail with an error that identifies the file as not being a tiramemsu database, and SHALL NOT create any tiramemsu table in it. An existing but completely empty SQLite file SHALL be initialised like a new file.

#### Scenario: Unrelated SQLite database
- **WHEN** a SQLite file containing only a table `customers` is opened
- **THEN** opening fails with a foreign-file error
- **AND** the file still contains only the table `customers`

#### Scenario: Empty SQLite file
- **WHEN** a zero-length file, or a SQLite file with no tables, is opened
- **THEN** it is initialised with the format-version-1 schema

### Requirement: One writer connection and a pool of readers
An open database SHALL use exactly one connection for all writes and a configurable number of additional read-only connections for reads, all on the same WAL file. Every read operation SHALL run inside a single read transaction so that it observes one consistent committed snapshot.

#### Scenario: Reads do not block on a running write
- **WHEN** a long transaction is in progress on the writer and another thread reads through a now view
- **THEN** the read completes without waiting for the write to finish
- **AND** it returns the state as of the last committed transaction

#### Scenario: Read snapshot is consistent
- **WHEN** a transaction that retracts one statement and asserts another commits while a single lookup is running
- **THEN** the lookup returns either both old values or both new values, never a mix
