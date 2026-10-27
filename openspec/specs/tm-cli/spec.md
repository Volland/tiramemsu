# tm-cli Specification

## Purpose
The `tm` command line creates tiramemsu files on a chosen backend, detects the backend of every file it opens, and inspects, queries, imports, exports, converts and compacts them.

## Requirements

### Requirement: Create on a chosen backend
`tm create <file> --backend <b>` SHALL create a new database on backend `b` (SQLite when omitted) and SHALL fail with exit code 1 and `AlreadyExists` when the file exists.

#### Scenario: Create a redb database
- **WHEN** `tm create mem.db --backend redb` runs on a missing path
- **THEN** the command exits 0 and `tm info mem.db` reports backend `redb`

### Requirement: Inspect a file
`tm info <file>` SHALL report the detected backend, the format version and the counts of live statements, all statements, terms and transactions.

#### Scenario: Info on a SQLite file
- **WHEN** `tm info` runs on a SQLite database with 3 statements, 1 of them retracted
- **THEN** it reports backend `sqlite`, 2 live statements and 3 statements

### Requirement: Query a file
`tm query <file> --sparql|--cypher <q>` SHALL run the query on the backend detected from the file and print the results in the requested format; `--as-of` and `--valid-at` SHALL select the view; `--explain` SHALL print the plan instead.

#### Scenario: Same query on both backends
- **WHEN** the same data is loaded into a SQLite and a redb file and the same SPARQL query runs on both with `--format json`
- **THEN** both outputs are identical

#### Scenario: Query error
- **WHEN** a query has a syntax error
- **THEN** the command exits 1 and stderr starts with `Parse:`

### Requirement: Import and export
`tm import` SHALL load N-Triples, N-Quads, Turtle and TriG files in chunked transactions; `tm export` SHALL write the live statements as N-Quads or N-Triples.

#### Scenario: Import then export
- **WHEN** an N-Triples file is imported into a new database and exported again
- **THEN** the exported triples equal the imported ones as a set

### Requirement: Convert and compact
`tm convert <src> <dst> --backend <b>` SHALL convert losslessly as `tiramemsu::convert` does, and `tm compact <file>` SHALL compact a redb file.

#### Scenario: Convert from SQLite to redb
- **WHEN** `tm convert a.sqlite b.redb --backend redb` runs
- **THEN** `tm info` on both files reports the same counts
