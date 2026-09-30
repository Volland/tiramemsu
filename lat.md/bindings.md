# Bindings

The Node.js and Python packages are thin wrappers over one Rust JSON bridge, so both behave the same and a new facade feature reaches both through a single change. The layout follows oxilite's bindings.

The bridge crate `tiramemsu-json` sits beside the two native crates under `bindings/`, and none of the three inherits the workspace's `unsafe_code = "forbid"`, because the napi and pyo3 macros expand to `unsafe`. See [[api#Bindings]] for the list of planned bindings.

## JSON Bridge

One `Database` object takes an operation name and a JSON object and returns JSON text, so each native crate is a constructor and a single `call` method, and every conversion lives in Rust once.

The bridge is [[bindings/json/src/lib.rs#Database]]. `call_text(op, args)` returns the result as JSON text, or an error as `{"code", "message"}` text. The native crates prefix that error text with `tiramemsu:` so a wrapper can parse it back into an exception with a code.

### Operations

Reads take a view and return rows; writes are one transaction each; anything that would surprise a caller is an error, never a silent default.

- **Reads:** `sparql`, `cypher`, `triples`, `path`, `events`, `graphs`, `graphMembers` and `values`, each with `{"view": …}` plus its own arguments. They are [[bindings/json/src/read.rs#run]]. `path` takes `start`, `path`, `mode`, `maxHops` and an optional `graphs` list of terms (a term that is not stored names no graph).
- **Writes:** `transact` (a list of op objects in one transaction), `cypherWrite`, and `with` (speculation: ops applied hypothetically, then queries run on the result, then everything discarded). They are in [[bindings/json/src/tx.rs#transact]].
- **Transaction ops:** `assert`, `create`, `retract`, `retractMatching`, `supersede`, `confirm`, `meta`, `upsert`, `newNode`, the five graph ops and `cypher`. An op may carry `"as": name`, and a later op may use `{"ref": name}` as a statement id or as a subject or object, which is how a layer is written on a statement created earlier in the same transaction.
- **Housekeeping:** `optimize` and `info`.

### Views

A view selects both clocks and is the same object for every read: `{"kind": "now" | "asOf" | "history", "tx", "instant", "validAt"}`.

`asOf` takes exactly one of `tx` (a transaction number) or `instant` (epoch milliseconds or an RFC 3339 string), and `validAt` may accompany any kind. Valid-time filtering is off unless `validAt` is given, as everywhere else. See [[query#Temporal Syntax]] and [[time-model]].

### Terms

A term is a JSON value: a string is a plain string, a number is an integer or a double, a boolean is a boolean, and an object with one key of `iri`, `node`, `bnode`, `stmt`, `tx`, `$int`, or `lex` with `datatype` or `lang` covers everything else.

What JSON cannot hold exactly comes back as `{"lex", "datatype"}`: integers beyond 2^53 use `{"$int"}`, and whole doubles, dates, date-times and decimals keep their canonical lexical form. So a value read from a result can be written back unchanged. The conversion is [[bindings/json/src/value.rs#value_to_json]] and its inverse `value_from_json`. Cypher results use the core's own `CypherValue::to_json`.

### Errors

Every failure has a code: the name of the core `Error` variant (`Parse`, `NotLive`, `UniqueViolation`, ...), or `InvalidArgument` when the bridge rejected the call itself.

A malformed op, an unknown operation, or a term of the wrong shape is `InvalidArgument`, and inside a transaction it fails the whole transaction, so nothing is committed. Codes are [[bindings/json/src/lib.rs#BindError]]`::code`.

## Node.js

`@tiramemsu/node` is a napi-rs addon plus a typed TypeScript wrapper, in `bindings/node`, built with `scripts/build-native.mjs` into `tiramemsu.<platform>-<arch>.node` and tested with vitest.

The native class is `Native(path, options)` with `call(op, args)`. The wrapper (`lib/index.ts`) adds `Database`, `View`, `Tx`, `Ref`, term helpers and `TiramemsuError`. It is synchronous, like the Rust API.

The package carries one addon per platform, and the loader names the platform when none matches. A `Date` is written as an `xsd:dateTime` literal and an `xsd:dateTime` is read back as a `Date`. Releases are built by `.github/workflows/npm-publish.yml`; see `docs/node-publishing.md`.

## Python

The `tiramemsu` package is a PyO3 module (`tiramemsu._native`, stable ABI) plus a typed Python wrapper, in `bindings/python`, built with maturin and checked with pytest and `mypy --strict`.

`Database.transact` is both a context manager, which records ops and submits them when the block exits cleanly, and a function of a list of op dicts. The GIL is released during each call, so threads can query one `Database` in parallel.

A `datetime` is written as an `xsd:dateTime` literal (a naive one is taken as UTC) and a `date` as `xsd:date`, and both are read back as the same Python types. `supersede` tells an omitted bound from `None`, which clears it. Wheels and the sdist are built by `.github/workflows/python-wheels.yml`; see `docs/python-publishing.md`.

## Test Strategy

Each binding runs the same story as the time-travel article against a real file, so the two packages and the article cannot drift apart.

The story is: Alice at Acme from 2020, corrected to end in 2024, then at Globex from March 2024, queried as of a transaction, valid at a date, both, and as history. Rust tests of the bridge itself are in `bindings/json/tests/bridge.rs`, and they are the reference the wrapper tests are written against.
