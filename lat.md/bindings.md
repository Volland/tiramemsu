# Bindings

The Node.js and Python packages are thin wrappers over one Rust JSON bridge, so both behave the same and a new facade feature reaches both through a single change. The layout follows oxilite's bindings.

The bridge crate `tiramemsu-json` sits beside the two native crates under `bindings/`, and none of the three inherits the workspace's `unsafe_code = "forbid"`, because the napi and pyo3 macros expand to `unsafe`. See [[api#Bindings]] for the list of planned bindings.

## JSON Bridge

One `Database` object takes an operation name and a JSON object and returns JSON text, so each native crate is a constructor and a single `call` method, and every conversion lives in Rust once.

The bridge is [[bindings/json/src/lib.rs#Database]]. `call_text(op, args)` returns the result as JSON text, or an error as `{"code", "message"}` text. The native crates prefix that error text with `tiramemsu:` so a wrapper can parse it back into an exception with a code.

### Operations

Reads take a view and return rows; writes are one transaction each; anything that would surprise a caller is an error, never a silent default.

- **Reads:** `sparql`, `cypher`, `triples`, `path`, `events`, `graphs`, `graphMembers`, `values`, `dependents` (`eid`, see [[time-model#Cascade#Dependents]]) and `bundle` (`eid`, returns the `tiramemsu-bundle/1` JSON of [[data-model#Fact Bundles#Bundle Formats]]), each with `{"view": …}` plus its own arguments. They are [[bindings/json/src/read.rs#run]]. `path` takes `start`, `path`, `mode`, `maxHops`, an optional `graphs` list of terms (a term that is not stored names no graph) and `timeRespecting: true | {"after": time}`, and every row carries `arrival` (epoch ms or `null`).
- **Writes:** `transact` (a list of op objects in one transaction), `cypherWrite`, and `with` (speculation: ops applied hypothetically, then queries run on the result, then everything discarded). They are in [[bindings/json/src/tx.rs#transact]].
- **Transaction ops:** `assert`, `create`, `retract`, `retractMatching`, `supersede`, `confirm`, `meta`, `upsert`, `newNode`, the five graph ops, `importBundle` (`bundle`; returns `{"root", "statements": [{"id", "eid", "new"}]}`, and `as` names the imported root) and `cypher`. An op may carry `"as": name`, and a later op may use `{"ref": name}` as a statement id or as a subject or object, which is how a layer is written on a statement created earlier in the same transaction.
- **Housekeeping:** `optimize`, `info` (with `importActive` and `statisticsDue`) and `cancel` (`key`, see [[bindings#JSON Bridge#Budgets]]).
- **Bulk import:** `importBegin`, `importChunk`, `importProgress`, `importFinish` and `importCancel`, see [[bindings#JSON Bridge#Bulk Import]].

### Budgets

Reads, `transact` and `cypherWrite` take an optional `budget` object that bounds the whole call, and `cancel` stops a running call by key from another thread.

The budget is `{"timeoutMs", "readerTimeoutMs", "maxRows", "maxBytes", "cancelKey"}`, all optional, mapped to a `QueryBudget` ([[query#Query Budgets]]); a read call runs under `QueryBudget::run`, so its term lookups, the read and decoding share one meter. An unknown key or a negative number is `InvalidArgument`. The open option `readerTimeoutMs` sets `OpenOptions::reader_timeout`.

`cancel` (`{"key"}`) cancels the token registered by the call running with that `cancelKey` and returns `{"running": bool}`. A key cancelled before its call starts makes that call fail at once, so callers use a fresh key per call. The key is released when its call ends.

### Bulk Import

The bridge keeps bulk import sessions by id, so a wrapper drives one with plain calls: begin, chunks, progress, then finish or cancel.

`importBegin` returns `{"session": n}` from `Db::bulk_import_shared` ([[query#Bulk Import]]). `importChunk` takes `session`, `ops`, `options` and `budget`, like `transact`, and returns the report with a `progress` member. Chunks of one session run one at a time.

Progress is `{"chunks", "rejected", "asserted", "existing", "retracted", "txs", "elapsedMs", "maintenanceMs"}`. `importFinish` returns `{"progress", "analyzed", "statisticsDue", "maintenanceError"}` (an error object or `null`); `importCancel` returns the progress. Both remove the session, and an unknown session is `InvalidArgument`.

### Views

A view selects both clocks and is the same object for every read: `{"kind": "now" | "asOf" | "history", "tx", "instant", "validAt"}`.

`asOf` takes exactly one of `tx` (a transaction number) or `instant` (epoch milliseconds or an RFC 3339 string), and `validAt` may accompany any kind. Valid-time filtering is off unless `validAt` is given, as everywhere else. See [[query#Temporal Syntax]] and [[time-model]].

### Terms

A term is a JSON value: a string is a plain string, a number is an integer or a double, a boolean is a boolean, and an object with one key of `iri`, `node`, `bnode`, `stmt`, `tx`, `$int`, or `lex` with `datatype` or `lang` covers everything else.

What JSON cannot hold exactly comes back as `{"lex", "datatype"}`: integers beyond 2^53 use `{"$int"}`, and whole doubles, dates, date-times and decimals keep their canonical lexical form. So a value read from a result can be written back unchanged. The conversion is [[bindings/json/src/value.rs#value_to_json]] and its inverse `value_from_json`. Cypher results use the core's own `CypherValue::to_json`.

### Errors

Every failure has a code: the name of the core `Error` variant (`Parse`, `NotLive`, `UniqueViolation`, `DeadlineExceeded`, `Cancelled`, `PoolTimeout`, `ResultLimitExceeded`, ...), or `InvalidArgument` when the bridge rejected the call itself.

A malformed op, an unknown operation, or a term of the wrong shape is `InvalidArgument`, and inside a transaction it fails the whole transaction, so nothing is committed. Codes are [[bindings/json/src/lib.rs#BindError]]`::code`.

## Node.js

`@tiramemsu/node` is a napi-rs addon plus a typed TypeScript wrapper, in `bindings/node`, built with `scripts/build-native.mjs` into `tiramemsu.<platform>-<arch>.node` and tested with vitest.

The native class is `Native(path, options)` with `call(op, args)`. The wrapper (`lib/index.ts`) adds `Database`, `View`, `Tx`, `Ref`, term helpers and `TiramemsuError`. It is synchronous, like the Rust API.

Every bridge operation has a wrapper method: `sparql(text, { provenance })`, `path` with `graphs` and `timeRespecting` (rows carry `arrival`), `dependents`, `bundle`, and `Tx.importBundle`.

Bulk import: `Database.bulkImport()` returns a `BulkImport` with `chunk(fn | ops, options)`, `progress()`, `finish()` and `cancel()`, and `info()` reports `importActive` and `statisticsDue`.

Budgets: `View.withBudget({ timeoutMs, readerTimeoutMs, maxRows, maxBytes, cancelKey })`, a `budget` member in the options of `transact` and `cypherWrite`, `Database.cancel(key)`, and the open option `readerTimeoutMs`. Because the API is synchronous, `cancel` from the same thread only affects a call that starts later.

The package carries one addon per platform, and the loader names the platform when none matches. A `Date` is written as an `xsd:dateTime` literal and an `xsd:dateTime` is read back as a `Date`. Releases are built by `.github/workflows/npm-publish.yml`; see `docs/node-publishing.md`.

## Python

The `tiramemsu` package is a PyO3 module (`tiramemsu._native`, stable ABI) plus a typed Python wrapper, in `bindings/python`, built with maturin and checked with pytest and `mypy --strict`.

Every bridge operation has a wrapper method, as in Node.js: `sparql(text, provenance=)`, `path(graphs=, time_respecting=)`, `dependents`, `bundle`, and `TxBuilder.import_bundle`, whose result is in `Report.results`.

Bulk import: `Database.bulk_import()` returns a `BulkImport` context manager (finish on clean exit, cancel on an exception) with `chunk()` in both `transact` forms, `progress()`, `finish()` and `cancel()`, and the dataclasses `ImportProgress` and `ImportSummary`.

Budgets: the frozen dataclass `QueryBudget(timeout_ms=, reader_timeout_ms=, max_rows=, max_bytes=, cancel_key=)`, `View.with_budget`, `budget=` on `transact` (both forms) and `cypher_write`, `Database(reader_timeout_ms=)`, and `Database.cancel(key)`, which works from another thread because the GIL is released during calls.

`Database.transact` is both a context manager, which records ops and submits them when the block exits cleanly, and a function of a list of op dicts. The GIL is released during each call, so threads can query one `Database` in parallel.

A `datetime` is written as an `xsd:dateTime` literal (a naive one is taken as UTC) and a `date` as `xsd:date`, and both are read back as the same Python types. `supersede` tells an omitted bound from `None`, which clears it. Wheels and the sdist are built by `.github/workflows/python-wheels.yml`; see `docs/python-publishing.md`.

## Test Strategy

Each binding runs the same story as the time-travel article against a real file, so the two packages and the article cannot drift apart.

The story is: Alice at Acme from 2020, corrected to end in 2024, then at Globex from March 2024, queried as of a transaction, valid at a date, both, and as history. Rust tests of the bridge itself are in `bindings/json/tests/bridge.rs`, and they are the reference the wrapper tests are written against.
