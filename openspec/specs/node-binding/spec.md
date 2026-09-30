# node-binding Specification

## Purpose
Defines `@tiramemsu/node`, the Node.js package: a napi-rs addon over the JSON bridge and a typed, synchronous TypeScript wrapper with `Database`, `View` and `Tx`.

## Requirements

### Requirement: Package layout and native surface

The package SHALL live in `bindings/node` with the crate `tiramemsu-node` (a `cdylib`), the npm name `@tiramemsu/node`, `type: module`, and one prebuilt addon per platform named `tiramemsu.<platform>-<arch>.node`. The native class SHALL be `Native` with a constructor taking a path and an options string, and one method `call(op, args)` over JSON text. A native failure SHALL be an `Error` whose message is `tiramemsu:` followed by the bridge's `{"code", "message"}` text. The crate SHALL NOT set the workspace lints.

#### Scenario: Build and load
- **WHEN** `npm run build:native` and `npm run build` are run
- **THEN** `tiramemsu.<platform>-<arch>.node` and `dist/index.js` exist and `import { Database } from "@tiramemsu/node"` loads

#### Scenario: Strict types
- **WHEN** `tsc --strict` is run on the package
- **THEN** it reports no errors

### Requirement: Database and views

`Database.open(path, options?)` SHALL open a database with the options of the bridge. `now()`, `asOf({tx} | {instant})` and `history()` SHALL return an immutable `View`, and `View.validAt(when)` SHALL return a new view with valid-time filtering, leaving the original unchanged. A `View` SHALL offer `sparql`, `cypher`, `triples`, `path`, `events`, `graphs`, `graphMembers` and `values`.

#### Scenario: Time travel on both clocks
- **WHEN** the Alice story (acme from 2020-01-01 with a confidence layer, superseded to end 2024-01-01, then globex from 2024-03-01) is run
- **THEN** `now()`, `asOf({tx: 1})`, `asOf({tx: 1}).validAt("2026-01-01")`, `now().validAt("2026-01-01")`, `now().validAt("2024-02-01")` and `history()` return the results the `json-bridge` scenarios list

#### Scenario: A view is immutable
- **WHEN** `const v = db.now(); const w = v.validAt("2026-01-01")`
- **THEN** a query on `v` is unfiltered by valid time and a query on `w` is filtered

#### Scenario: Persistence
- **WHEN** a database file is closed by dropping the object and reopened
- **THEN** every statement and its history are still there

### Requirement: Transactions with references

`db.transact(fn, options?)` SHALL call `fn` with a `Tx` that records ops, submit them once when `fn` returns, and return the report. The `Tx` methods that create a statement (`assert`, `create`, `supersede`) SHALL return a `Ref` that later calls in the same callback accept as a statement id, subject or object. If `fn` throws, nothing SHALL be submitted. `transact` SHALL also accept a plain op array. `cypherWrite`, `speculate` and `optimize` SHALL be available on `Database`.

#### Scenario: Layers by reference
- **WHEN** `const job = tx.assert(alice, worksAt, acme); tx.assert(job, confidence, 0.8)` runs in one callback
- **THEN** a Cypher read returns the confidence as a property of the relationship, and a SPARQL query with `{| |}` returns it as an annotation

#### Scenario: A throwing callback commits nothing
- **WHEN** the callback asserts a statement and then throws
- **THEN** the error propagates and `triples` returns no rows

#### Scenario: Speculation
- **WHEN** `speculate` applies an assert and runs a query
- **THEN** the query sees the statement and the database is unchanged afterwards

#### Scenario: Dry run
- **WHEN** `transact(fn, {dryRun: true})` returns
- **THEN** the report lists the assert and the database is unchanged

### Requirement: Terms map to JavaScript

A JavaScript `string` SHALL be a plain string, an integer `number` an `xsd:integer`, a fractional `number` an `xsd:double`, a `boolean` a boolean, a `bigint` an integer of any size, and a `Date` an `xsd:dateTime`. `iri`, `literal`, `node`, `bnode`, `stmt` and `tx` helpers SHALL build the tagged terms. Results SHALL decode to the same shapes, integers beyond 2^53 as `bigint` and whole doubles as `number`.

#### Scenario: Big integer round trip
- **WHEN** `9007199254740993n` is asserted and read back
- **THEN** the result is the `bigint` `9007199254740993n`

#### Scenario: Date round trip
- **WHEN** a `Date` is asserted as an object and read back
- **THEN** the result is a `Date` with the same instant

### Requirement: Errors have codes

Every native failure SHALL throw a `TiramemsuError` with a string `code` (the bridge code) and a `message`.

#### Scenario: Parse error
- **WHEN** `sparql("SELECT ?")` is called
- **THEN** it throws a `TiramemsuError` with `code` `Parse`

#### Scenario: Not live
- **WHEN** `confirm` names an eid that is not live
- **THEN** it throws a `TiramemsuError` with `code` `NotLive`

### Requirement: Events, paths and graphs

`events(since)` SHALL list the change log after a transaction, `path(start, path, {mode, maxHops})` SHALL return endpoints with hop counts, and the graph methods SHALL expose named graphs, each as the bridge specifies.

#### Scenario: Reachability
- **WHEN** a knows b and b knows c, and `path(a, "knows+")` runs
- **THEN** it returns two rows

#### Scenario: Named graph
- **WHEN** a statement is added to a graph
- **THEN** `graphs()` lists it and `graphMembers(g)` lists the statement
