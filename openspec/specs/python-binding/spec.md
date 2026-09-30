# python-binding Specification

## Purpose
Defines the `tiramemsu` Python package: a PyO3 module over the JSON bridge on the stable ABI, and a typed synchronous wrapper with `Database`, `View` and `Tx`.

## Requirements

### Requirement: Package layout and native surface

The package SHALL live in `bindings/python` with the crate `tiramemsu-python` (a `cdylib`), the PyPI name `tiramemsu`, the native module `tiramemsu._native` on the stable ABI (`abi3-py39`), a `py.typed` marker and type stubs. The native class SHALL be `Native` with a constructor taking a path and an options string, and one method `call(op, args)` over JSON text that releases the GIL. A native failure SHALL be a `RuntimeError` whose message is `tiramemsu:` followed by the bridge's `{"code", "message"}` text. The package version SHALL come from the Cargo workspace. The crate SHALL NOT set the workspace lints.

#### Scenario: Build and import
- **WHEN** `maturin develop` is run in a virtual environment
- **THEN** `import tiramemsu` succeeds and `tiramemsu.__version__` is the workspace version

#### Scenario: Strict types
- **WHEN** `mypy --strict python/tiramemsu` is run
- **THEN** it reports no errors

### Requirement: Database and views

`Database(path, **options)` SHALL open a database with the options of the bridge and SHALL work as a context manager. `now()`, `as_of(tx=… | instant=…)` and `history()` SHALL return an immutable `View`, and `View.valid_at(when)` SHALL return a new view. A `View` SHALL offer `sparql`, `cypher`, `triples`, `path`, `events`, `graphs`, `graph_members` and `values`.

#### Scenario: Time travel on both clocks
- **WHEN** the Alice story is run
- **THEN** the six views return the results the `json-bridge` scenarios list

#### Scenario: Persistence
- **WHEN** a database file in a temporary directory is reopened
- **THEN** every statement and its history are still there

#### Scenario: Threads
- **WHEN** several Python threads query one `Database` at the same time
- **THEN** every query returns the correct rows and none blocks the others for the length of a query

### Requirement: Transactions as a context manager or a list

`db.transact()` SHALL be a context manager that records ops and submits them once when the block exits cleanly, with the report on `tx.report`. If the block raises, nothing SHALL be submitted. `db.transact(ops)` SHALL accept a list of op dicts and return the report. `Tx` methods that create a statement SHALL return a `Ref` that later calls in the same block accept as a statement id, subject or object. `supersede` SHALL distinguish an omitted bound from a `None` bound that clears it. `cypher_write`, `speculate` and `optimize` SHALL be available.

#### Scenario: Layers by reference
- **WHEN** `job = tx.assert_(alice, works_at, acme)` and `tx.assert_(job, confidence, 0.8)` run in one block
- **THEN** a Cypher read returns the confidence as a relationship property, and a SPARQL query with `{| |}` returns it

#### Scenario: A raising block commits nothing
- **WHEN** a block asserts a statement and then raises
- **THEN** the exception propagates and `triples()` returns no rows

#### Scenario: Clearing a bound
- **WHEN** `supersede(job, valid_to=None)` is called on a statement with a `valid_to`
- **THEN** the replacement has no end, and `supersede(job)` with no bound keeps the old end

#### Scenario: Speculation and dry run
- **WHEN** `speculate` applies an assert and queries it, and `transact(dry_run=True)` is used
- **THEN** the query sees the statement, the report lists it, and the database is unchanged afterwards

### Requirement: Terms map to Python

A `str` SHALL be a plain string, an `int` an `xsd:integer` of any size, a `float` an `xsd:double`, a `bool` a boolean, and a `datetime.datetime` or `datetime.date` an `xsd:dateTime` or `xsd:date`. Frozen dataclasses `Iri`, `Literal`, `Node`, `BNode`, `Stmt` and `Tx` SHALL cover the rest. Results SHALL decode to the same types, and integers SHALL round-trip exactly.

#### Scenario: Big integer round trip
- **WHEN** `2**63 - 1` is asserted and read back
- **THEN** the result is the `int` `2**63 - 1`

#### Scenario: Datetime round trip
- **WHEN** an aware `datetime` is asserted and read back
- **THEN** the result is a `datetime` with the same instant

### Requirement: Errors have codes

Every native failure SHALL raise `TiramemsuError` with a string `code` (the bridge code) and a message.

#### Scenario: Parse error
- **WHEN** `view.sparql("SELECT ?")` is called
- **THEN** it raises `TiramemsuError` with `code == "Parse"`

#### Scenario: Not live
- **WHEN** `confirm` names an eid that is not live
- **THEN** it raises `TiramemsuError` with `code == "NotLive"`

### Requirement: Query results are typed

`sparql` SHALL return a result object per kind (select rows as dicts of variable to term with a `vars` attribute, ask as a boolean, graph as triples, update as a report). `cypher` SHALL return a result with `columns` and `rows`. `triples` SHALL return dataclasses with `eid`, `s`, `p`, `o`, `t_add`, `t_ret`, `valid_from`, `valid_to` and `ret_kind`.

#### Scenario: Select rows
- **WHEN** a SELECT binds one variable over one statement
- **THEN** iterating the result yields one dict and `vars` lists the variable

#### Scenario: Statement rows
- **WHEN** `history().triples(s=alice, p=works_at, o=acme)` runs on the Alice data
- **THEN** it returns two `Statement` values, one with `ret_kind == "supersede"`
