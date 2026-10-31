## MODIFIED Requirements

### Requirement: Import and export
`tm import` SHALL load N-Triples, N-Quads, Turtle and TriG files in chunked transactions; `tm export` SHALL write the live statements as N-Quads or N-Triples, and with `--format sg-nt12` every statement as an RDF 1.2 statement graph (see *Statement graph export*).

#### Scenario: Import then export
- **WHEN** an N-Triples file is imported into a new database and exported again
- **THEN** the exported triples equal the imported ones as a set

## ADDED Requirements

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
