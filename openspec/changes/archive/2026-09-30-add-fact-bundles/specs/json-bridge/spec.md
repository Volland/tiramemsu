## MODIFIED Requirements

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
