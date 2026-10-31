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
`tm import` SHALL load N-Triples, N-Quads, Turtle and TriG files in chunked transactions; `tm export` SHALL write the live statements as N-Quads or N-Triples, and with `--format sg-nt12` every statement as an RDF 1.2 statement graph (see *Statement graph export*).

#### Scenario: Import then export
- **WHEN** an N-Triples file is imported into a new database and exported again
- **THEN** the exported triples equal the imported ones as a set

### Requirement: Convert and compact
`tm convert <src> <dst> --backend <b>` SHALL convert losslessly as `tiramemsu::convert` does, and `tm compact <file>` SHALL compact a redb file.

#### Scenario: Convert from SQLite to redb
- **WHEN** `tm convert a.sqlite b.redb --backend redb` runs
- **THEN** `tm info` on both files reports the same counts

### Requirement: Statement graph export
`tm export <file> --format sg-nt12 [--as-of T | --history] [--valid-at MS]` SHALL write the statements of the file in LeanRDF's statement-graph format (`leanrdf-statements`, the `statement-graphs` capability of LeanRDF v0.3.1). For every statement `e` whose predicate is not a `sys:` predicate and that was not added and retracted in one transaction, it SHALL write:
- `<urn:tiramemsu:stmt:e> rdf:reifies <<( s p o )>>`, with statements in `s` and `o` written as `<urn:tiramemsu:stmt:n>`;
- `tm:txAdded`, and `tm:txRetracted` when retracted, as `xsd:integer` transaction numbers;
- `tm:validFromMs` and `tm:validToMs`, when present, as `xsd:integer` epoch milliseconds;
- the base triple `s p o` exactly when `e` is live in the view: `now` by default, `--as-of T`, or `--history`, restricted by `--valid-at MS` when given.

The output SHALL be canonical RDF 1.2 N-Triples in LeanRDF's triple order, so that `leanrdf-sg decode` accepts it in the same view and `leanrdf-sg export` of the result reproduces it byte for byte. Statement content that uses `urn:tiramemsu:stmt:` IRIs as plain IRIs, or `rdf:reifies` as a predicate, SHALL be refused with an error. The view flags SHALL be refused with any other format. The format is export-only: `tm import` does not read it back.

#### Scenario: A route and its distance
- **WHEN** `(:FRA :route :LHR)` is added in transaction 1 and `(e1 :distance 655)` in transaction 2, and the file is exported in `now` and in `--history`
- **THEN** both outputs equal LeanRDF's golden `routes.now.nt` and `routes.history.nt` byte for byte

#### Scenario: A correction in four views
- **WHEN** a distance added in transaction 2 is superseded in transaction 4, a sourced statement is added about the correction, and a route has a valid interval, and the file is exported in `now`, `--as-of 3`, `--history` and `--valid-at 1800000000000`
- **THEN** each output matches its recorded file, the superseded statement is reified with `tm:txRetracted 4` and asserted only as of 3 and in history, the engine's `sys:supersedes` statement is left out, and LeanRDF decodes every file and re-exports it byte for byte

#### Scenario: Never-visible statements are left out
- **WHEN** a statement is added and retracted in the same transaction
- **THEN** the history export has no reifier for it

#### Scenario: Canonical RDF 1.2 N-Triples
- **WHEN** literals with a tab, a quote, BACKSPACE, FORM FEED, U+0001 and U+007F, a language tag in upper case, a boolean and a decimal are exported
- **THEN** the output uses `\t`, `\"`, `\b`, `\f`, `\u0001` and `\u007F`, a lower-case tag and typed literals, as LeanRDF's serializer does
