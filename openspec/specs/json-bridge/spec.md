# json-bridge Specification

## Purpose
Defines the shared JSON call surface that the Node.js and Python packages wrap. One `Database` takes an operation name and a JSON object and returns JSON, so both packages expose the same operations, terms, views and errors.

## Requirements

### Requirement: One call surface

The bridge SHALL expose `Database::open(path, options)` and `Database::call(op, args)`, and `call_text(op, args)` over JSON text. `call` SHALL accept the reads `sparql`, `cypher`, `triples`, `path`, `events`, `graphs`, `graphMembers`, `values`, `dependents` and `bundle`, the writes `transact`, `cypherWrite` and `with`, and `optimize` and `info`. An unknown operation SHALL fail with code `InvalidArgument`. `open` SHALL accept only the options `readers`, `busyTimeoutMs`, `termCacheCapacity`, `optimizeEvery`, `pathMaxHops` and `pathMaxStates`, and SHALL reject any other key with `InvalidArgument`.

#### Scenario: Unknown operation
- **WHEN** `call("nope", {})` is called
- **THEN** it fails with code `InvalidArgument`

#### Scenario: Unknown open option
- **WHEN** `Database::open(path, {"bogus": 1})` is called
- **THEN** it fails with code `InvalidArgument` and creates no database

#### Scenario: Reader count option
- **WHEN** a database is opened with `{"readers": 2}` and `call("info")` is called
- **THEN** the result has `readers` equal to 2

#### Scenario: Dependents and bundle reads
- **WHEN** a statement with one layer exists and `call("dependents", {"eid": e})` and `call("bundle", {"eid": e})` are called
- **THEN** the first returns two eids, the root first, and the second returns an object whose `format` is `tiramemsu-bundle/1` and which holds two statements

### Requirement: Views select both clocks

Every read SHALL take a view `{"kind": "now" | "asOf" | "history", "tx", "instant", "validAt"}`, with `null` or an absent view meaning `now`. An `asOf` view SHALL carry exactly one of `tx` (a transaction number) and `instant` (epoch milliseconds or an RFC 3339 date or date-time), and SHALL otherwise fail with `InvalidArgument`. `validAt` MAY accompany any kind. Valid-time filtering SHALL be off unless `validAt` is present.

#### Scenario: As of a transaction
- **WHEN** alice worksAt acme is asserted in transaction 1, superseded in transaction 2 with a `validTo` of 2024-01-01, and globex is asserted in transaction 3
- **THEN** a SPARQL query on `{"kind": "asOf", "tx": 1}` returns acme only, and on `{"kind": "now"}` returns acme and globex

#### Scenario: Both clocks together
- **WHEN** the same data is queried on `{"kind": "asOf", "tx": 1, "validAt": "2026-01-01"}`
- **THEN** it returns acme, and on `{"kind": "now", "validAt": "2026-01-01"}` it returns globex, and on `{"kind": "now", "validAt": "2024-02-01"}` it returns nothing

#### Scenario: History shows retracted statements
- **WHEN** the `triples` operation runs on the history view for alice worksAt acme
- **THEN** it returns two rows, one of them with `retKind` `supersede` and `tRet` 2

#### Scenario: Both selectors on one as-of view
- **WHEN** a view has `kind` `asOf` and neither `tx` nor `instant`
- **THEN** the call fails with `InvalidArgument`

### Requirement: Terms are exact in both directions

A term SHALL be written as: a JSON string for a plain string, a boolean, a JSON integer for an `xsd:integer`, a fractional number for an `xsd:double`, or an object with one key of `iri`, `node`, `bnode`, `stmt`, `tx`, `$int`, or `lex` with `datatype` or `lang`. A term that JSON cannot carry exactly (an integer beyond 2^53, a whole double, a date, a date-time or a decimal) SHALL be returned as `{"lex", "datatype"}` and SHALL be accepted back unchanged.

#### Scenario: Integer beyond 2^53
- **WHEN** the object `{"$int": "9007199254740993"}` is asserted and read back with `triples`
- **THEN** the object is returned as `{"$int": "9007199254740993"}`

#### Scenario: Whole double keeps its datatype
- **WHEN** `{"lex": "3", "datatype": "http://www.w3.org/2001/XMLSchema#double"}` is asserted and read back
- **THEN** the datatype is `xsd:double` and the lexical form is the canonical `3.0E0`

#### Scenario: Language-tagged string
- **WHEN** `{"lex": "chat", "lang": "fr"}` is asserted and read back
- **THEN** it is returned as `{"lex": "chat", "lang": "fr"}`

#### Scenario: A term that is not a term
- **WHEN** an assert names `null` or a list as its object
- **THEN** the call fails with `InvalidArgument`

### Requirement: Transactions are op lists with named references

`transact` SHALL apply a list of op objects in one transaction and return the transaction report with `results` (one per op) and `refs` (the statement ids named with `as`). The ops SHALL be `assert`, `create`, `retract`, `retractMatching`, `supersede`, `confirm`, `meta`, `upsert`, `newNode`, `addToGraph`, `removeFromGraph`, `clearGraph`, `createGraph`, `dropGraph`, `importBundle` and `cypher`. An op MAY carry `"as": name`, and a later op SHALL accept `{"ref": name}` as a statement id and as a subject or object; for `importBundle` the name refers to the imported root. Times (`validFrom`, `validTo`) SHALL accept epoch milliseconds or an RFC 3339 date or date-time. If any op fails, nothing SHALL be committed.

#### Scenario: A layer on a statement created in the same transaction
- **WHEN** `assert alice worksAt acme` is submitted with `"as": "job"` followed by an assert whose subject is `{"ref": "job"}`
- **THEN** both are committed in transaction 1 and `refs.job` equals the eid of the first result

#### Scenario: Idempotent assert
- **WHEN** the same assert is submitted twice in one transaction
- **THEN** the second result has `new` false

#### Scenario: Failure commits nothing
- **WHEN** a transaction holds a valid assert followed by an op named `bogus`
- **THEN** it fails with `InvalidArgument` and `triples` returns no rows

#### Scenario: Retract cascades to layers
- **WHEN** a statement with a confidence layer is retracted
- **THEN** the report lists the statement as `explicit` and the layer as `cascade`, and the event log holds two retract events

#### Scenario: Dry run
- **WHEN** a transaction is submitted with `{"dryRun": true}`
- **THEN** the report lists the assert and nothing is stored

#### Scenario: Unknown eid on retract
- **WHEN** `retract` names an eid that does not exist
- **THEN** the result for that op is `false` and the transaction commits

#### Scenario: Import a bundle
- **WHEN** `{"op": "importBundle", "bundle": b, "as": "fact"}` is submitted with `b` read by `bundle` from another database
- **THEN** the result maps every bundle id to an eid with a `new` flag, and `refs.fact` is the eid of the imported root

### Requirement: Speculation keeps nothing

`with` SHALL apply an op list hypothetically, run each query of `queries` on the resulting state, return the results, and keep nothing, so the event log and the triples are unchanged afterwards.

#### Scenario: What-if
- **WHEN** `with` applies an assert and runs `triples` and `ASK` on it
- **THEN** the results show the statement, and afterwards `triples` on now returns no rows and `events` returns none

### Requirement: Errors carry a code

Every failure SHALL be `{"code", "message"}`, where `code` is the name of the core error variant (for example `Parse`, `Unsupported`, `NotLive`, `UniqueViolation`, `SubjectTypeMismatch`) or `InvalidArgument` when the bridge rejected the call itself. `call_text` SHALL return the failure as that JSON text.

#### Scenario: Parse error
- **WHEN** `sparql` runs the text `SELECT ?`
- **THEN** it fails with code `Parse`, and `call_text` returns text containing `"code":"Parse"`

#### Scenario: Write through a read-only view
- **WHEN** `cypher` runs `CREATE (:Person)`
- **THEN** it fails with code `Unsupported`

#### Scenario: Missing argument
- **WHEN** an `assert` op has no `p` and no `o`
- **THEN** it fails with code `InvalidArgument`

#### Scenario: Subject type violation
- **WHEN** `v:confidence` has `sys:subjectType sys:STMT` and a `transact` call asserts `(v:alice v:confidence 0.8)`
- **THEN** it fails with code `SubjectTypeMismatch`, and nothing from that call is committed

### Requirement: Query results

`sparql` SHALL return `{"kind": "select", "vars", "rows"}` with each row an object of the bound variables, `{"kind": "ask", "value"}`, `{"kind": "graph", "triples"}` or `{"kind": "update", "report"}`. `sparql` SHALL accept an optional boolean `provenance`; when it is `true`, a select result SHALL also have `"provenance"`, an array parallel to `"rows"` whose entries list the row's statements as `{"stmt": n}` in ascending order, and the other forms SHALL fail with code `Unsupported`. `cypher` SHALL return `{"columns", "rows"}`. `triples` SHALL return one object per statement with its eid, its three terms, `tAdd`, `tRet`, `validFrom`, `validTo` and `retKind`. A `triples` pattern that names a term that is not stored SHALL return no rows and SHALL NOT fail.

#### Scenario: Cypher rows
- **WHEN** a Cypher write creates two people linked by KNOWS and a read returns both names
- **THEN** the result has columns `["a", "b"]` and one row `["Alice", "Bob"]`

#### Scenario: Unstored term
- **WHEN** `triples` is called with a subject that was never asserted
- **THEN** it returns an empty list

#### Scenario: SPARQL rows with provenance
- **WHEN** `sparql` runs `SELECT ?o WHERE { v:a v:p ?o }` with `"provenance": true` on a store where `(v:a v:p v:b)` is statement 1
- **THEN** the result has `"rows": [{"o": …}]` and `"provenance": [[{"stmt": 1}]]`, and without the argument it has no `"provenance"` member

### Requirement: Paths and graphs

`path` SHALL take `start`, `path` text, `mode` (`reach`, `trail`, `anyShortest`, `allShortest`), `maxHops`, an optional `graphs` list of terms and an optional `timeRespecting` (`true`, or `{"after": <time>}` with a time as the bridge encodes times), and return one row per endpoint with its hop count, its `arrival` (epoch ms, or `null` when the search is not time-respecting or the arrival is unbounded) and, except in `reach` mode, its nodes and hops. With `graphs`, the path SHALL be evaluated with graph-scoped evaluation over those graphs; a listed term that is not stored names no graph. With `timeRespecting`, the path SHALL be evaluated time-respecting. `addToGraph`, `graphs` and `graphMembers` SHALL expose named graphs as tags on statements.

#### Scenario: Reachability
- **WHEN** a knows b and b knows c, and `path` runs `knows+` from a
- **THEN** it returns two rows

#### Scenario: A graph holds a statement
- **WHEN** a statement is added to graph session12
- **THEN** `graphs` returns session12 and `graphMembers` returns the statement's eid

#### Scenario: Path inside a graph
- **WHEN** a knows b is in graph session12, b knows c is in no graph, and `path` runs `knows+` from a with `"graphs": [session12]`
- **THEN** it returns one row, for b

#### Scenario: Time-respecting path
- **WHEN** a met b valid `[1, 5)`, b met c valid `[3, 9)`, and `path` runs `met+` from a with `"timeRespecting": true`
- **THEN** it returns b with `"arrival": 1` and c with `"arrival": 3`, and with `"timeRespecting": {"after": 6}` it returns no row
