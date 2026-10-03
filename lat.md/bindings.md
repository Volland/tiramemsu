# Bindings

The Node.js and Python packages are thin wrappers over one Rust JSON bridge, so both behave the same and a new facade feature reaches both through a single change. The layout follows oxilite's bindings.

The bridge crate `tiramemsu-json` sits beside the two native crates under `bindings/` and is publishable, because the MCP server ([[api#MCP Tools]]) is built on it too; and none of the three inherits the workspace's `unsafe_code = "forbid"`, because the napi and pyo3 macros expand to `unsafe`. See [[api#Bindings]] for the list of planned bindings.

## JSON Bridge

One `Database` object takes an operation name and a JSON object and returns JSON text, so each native crate is a constructor and a single `call` method, and every conversion lives in Rust once.

The bridge is [[bindings/json/src/lib.rs#Database]]. `call_text(op, args)` returns the result as JSON text, or an error as `{"code", "message"}` text. The native crates prefix that error text with `tiramemsu:` so a wrapper can parse it back into an exception with a code.

### Operations

Reads take a view and return rows; writes are one transaction each; anything that would surprise a caller is an error, never a silent default.

- **Reads:** `sparql` (`text`, `provenance`, `queryOnly`, `params`, `pathCompleteness`; with provenance the result adds `provenance` and `provenanceGaps`), `cypher` (`text`, `params`, `pathCompleteness`), `triples`, `path`, `events`, `graphs`, `graphMembers`, `values`, `dependents` (`eid`, see [[time-model#Cascade#Dependents]]) and `bundle` (`eid`, returns the `tiramemsu-bundle/1` JSON of [[data-model#Fact Bundles#Bundle Formats]]), each with `{"view": …}` plus its own arguments. They are [[bindings/json/src/read.rs#run]]. `path` takes `start`, `path`, `mode`, `maxHops`, an optional `graphs` list of terms (a term that is not stored names no graph) and `timeRespecting: true | {"after": time}`, and every row carries `arrival` (epoch ms or `null`). `path` also takes `capped: true` (stop an unbounded search at `pathMaxHops`) and `completeness: true`, which returns `{"rows": [...], "completeness": …}` instead of the plain array.
- **Temporal paths and completeness:** `sparql` `params` is `{"name": time}` (epoch ms or an RFC 3339 date or date-time), the starts of `SERVICE <urn:tiramemsu:tm:timeRespecting/$name>` ([[query#Temporal Path Syntax]]). With `pathCompleteness: true`, a `select` result and a `cypher` result gain `"pathCompleteness": {"kind": "exhaustive" | "bound" | "cap", "maxHops": n | null, "complete": bool}` or `null` when no path ran; without it the responses are unchanged.
- **Writes:** `transact` (a list of op objects in one transaction), `cypherWrite`, and `with` (speculation: ops applied hypothetically, then queries run on the result, then everything discarded). They are in [[bindings/json/src/tx.rs#transact]].
- **Transaction ops:** `assert`, `create`, `retract`, `retractMatching`, `supersede`, `confirm`, `meta`, `upsert`, `newNode`, the five graph ops, `importBundle` (`bundle`; returns `{"root", "statements": [{"id", "eid", "new"}]}`, and `as` names the imported root) and `cypher`. An op may carry `"as": name`, and a later op may use `{"ref": name}` as a statement id or as a subject or object, which is how a layer is written on a statement created earlier in the same transaction.
- **Explain:** `explainSparql` (`text`, plus `view` and `budget`), see [[bindings#JSON Bridge#Explain]].
- **Housekeeping:** `optimize`, `info` (with `importActive` and `statisticsDue`), `cancel` (`key`, see [[bindings#JSON Bridge#Budgets]]), `rebuildTextIndex` and `enableTextIndex` (see [[bindings#JSON Bridge#Text Recall]]).
- **Bulk import:** `importBegin`, `importChunk`, `importProgress`, `importFinish` and `importCancel`, see [[bindings#JSON Bridge#Bulk Import]].
- **Saved answers:** `saveAnswer`, `savedAnswer`, `savedAnswers`, `checkSavedAnswers`, `refreshAnswer` and `deleteSavedAnswer`, see [[bindings#JSON Bridge#Saved Answers]].

### Budgets

Reads, `transact` and `cypherWrite` take an optional `budget` object that bounds the whole call, and `cancel` stops a running call by key from another thread.

The budget is `{"timeoutMs", "readerTimeoutMs", "maxRows", "maxBytes", "cancelKey"}`, all optional, mapped to a `QueryBudget` ([[query#Query Budgets]]); a read call runs under `QueryBudget::run`, so its term lookups, the read and decoding share one meter. An unknown key or a negative number is `InvalidArgument`. The open option `readerTimeoutMs` sets `OpenOptions::reader_timeout`.

`cancel` (`{"key"}`) cancels the token registered by the call running with that `cancelKey` and returns `{"running": bool}`. A key cancelled before its call starts makes that call fail at once, so callers use a fresh key per call. The key is released when its call ends.

### Bulk Import

The bridge keeps bulk import sessions by id, so a wrapper drives one with plain calls: begin, chunks, progress, then finish or cancel.

`importBegin` returns `{"session": n}` from `Db::bulk_import_shared` ([[query#Bulk Import]]). `importChunk` takes `session`, `ops`, `options` and `budget`, like `transact`, and returns the report with a `progress` member. Chunks of one session run one at a time.

Progress is `{"chunks", "rejected", "asserted", "existing", "retracted", "txs", "elapsedMs", "maintenanceMs"}`. `importFinish` returns `{"progress", "analyzed", "statisticsDue", "maintenanceError"}` (an error object or `null`); `importCancel` returns the progress. Both remove the session, and an unknown session is `InvalidArgument`.

### Text Recall

`textSearch` is a read like the others, so it takes a view and a budget, and returns ranked hits with their evidence ([[query#Text Recall]]).

Arguments are `text`, `mode` (`"all"`, `"any"`, `"phrase"`), `graphs` and `predicates` (lists of terms; a term that is not stored matches nothing), `limit` and `confidence` (a predicate term). Each hit is `{"eid", "s", "p", "o", "text", "lang", "score", "rank", "evidence": {"confidence", "confirmations", "authors", "tAdd", "addedAt"}}`, with an absent confidence as `null`.

The open option `textIndex: true` builds the index at open. `rebuildTextIndex` returns `{"values": n}` and `enableTextIndex` returns `{"built": bool}`. Errors are `TextIndexUnavailable` and `MissingCapability`; an unknown option is `InvalidArgument`. Node's `View.sparql` takes `{ provenance, queryOnly }` and returns `provenanceGaps`; Python's `View.sparql` takes `provenance` and `query_only` and returns `provenance_gaps`. Node exposes `View.textSearch`, `Database.rebuildTextIndex` and `enableTextIndex`; Python `View.text_search`, `Database.rebuild_text_index` and `enable_text_index`.

### Explain

`explainSparql` explains SPARQL query text without running it, so a caller can see whether a cyclic pattern took the native cyclic-join route ([[query#Physical Planning#LFTJ]]) and why not.

It returns `{"regions": [{"kind", "note", "aliases", "queryPlan"}], "sql", "shortCircuit", "queryPlan"}` from [[bindings/json/src/read.rs#explain_json]]. Kinds are `sql`, `nativePath` and `nativeLftj`; notes are `none`, `cyclicLftjDisabled`, `lftjUnavailable`, `lftjUnsupportedShape`, `lftjBelowEstimate`, `lftjNative`, `pathForward` and `pathInverted`. An update is `Unsupported`.

The open options `lftj` (boolean) and `lftjMinRows` (non-negative integer) set `OpenOptions::planner.lftj`. Node exposes `Database.open(path, { lftj, lftjMinRows })` and `View.explainSparql(text)` with `Explain` and `ExplainRegion` types; Python `Database(path, lftj=, lftj_min_rows=)` and `View.explain_sparql(text)`, which returns the dictionary.

### Saved Answers

The saved-answer calls of [[query#Saved Answers]], in [[bindings/json/src/saved.rs#run]]: save a query, check events into marks once, refresh, read and delete.

`saveAnswer` takes `name`, `text`, `language` (`"sparql"` default, or `"cypher"`), `params`, `view` and `budget`; `refreshAnswer` takes `name` and `budget`; `savedAnswer` and `deleteSavedAnswer` take `name`. An answer is `{"name", "language", "text", "params", "view", "vocab", "prefixes", "result", "dependencies", "coverage", "checkpoint", "cursor", "evaluatedAt", "revision", "status", "invalidation", "error"}`, where `result` has the shape of a live `sparql` or `cypher` call. `checkSavedAnswers` returns the new invalidations `{"name", "status", "cause", "t", "event"}`, and an unknown name on refresh is `SavedAnswerNotFound`.

Node exposes `Database.saveAnswer(name, {text, language, params, view}, budget)`, `savedAnswer`, `savedAnswers`, `checkSavedAnswers`, `refreshAnswer` and `deleteSavedAnswer`; Python `Database.save_answer(name, text, language=, params=, view=, budget=)`, `saved_answer`, `saved_answers`, `check_saved_answers`, `refresh_answer` and `delete_saved_answer`, returning `SavedAnswer` and `Invalidation` dataclasses.

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

Temporal paths: `sparql(text, { params, pathCompleteness })`, `cypher(text, params, { pathCompleteness })`, `PathOptions.capped` and `View.pathReport(start, expr, opts)` returning `{ rows, completeness }` with the typed `PathCompleteness`.

Bulk import: `Database.bulkImport()` returns a `BulkImport` with `chunk(fn | ops, options)`, `progress()`, `finish()` and `cancel()`, and `info()` reports `importActive` and `statisticsDue`.

Budgets: `View.withBudget({ timeoutMs, readerTimeoutMs, maxRows, maxBytes, cancelKey })`, a `budget` member in the options of `transact` and `cypherWrite`, `Database.cancel(key)`, and the open option `readerTimeoutMs`. Because the API is synchronous, `cancel` from the same thread only affects a call that starts later.

Text recall: the open option `textIndex`, `View.textSearch(text, { mode, graphs, predicates, limit, confidence })` returning typed `TextHit`s, `Database.rebuildTextIndex()` and `Database.enableTextIndex()`.

The package carries one addon per platform, and the loader names the platform when none matches. A `Date` is written as an `xsd:dateTime` literal and an `xsd:dateTime` is read back as a `Date`. Releases are built by `.github/workflows/npm-publish.yml`; see `docs/node-publishing.md`.

## Python

The `tiramemsu` package is a PyO3 module (`tiramemsu._native`, stable ABI) plus a typed Python wrapper, in `bindings/python`, built with maturin and checked with pytest and `mypy --strict`.

Every bridge operation has a wrapper method, as in Node.js: `sparql(text, provenance=)`, `path(graphs=, time_respecting=)`, `dependents`, `bundle`, and `TxBuilder.import_bundle`, whose result is in `Report.results`.

Temporal paths: `sparql(text, params=, path_completeness=)`, `cypher(text, params, path_completeness=)`, `path(capped=)` and `View.path_report(...)` returning a `PathReport(rows, completeness)` with the frozen dataclass `PathCompleteness(kind, max_hops, complete)`.

Bulk import: `Database.bulk_import()` returns a `BulkImport` context manager (finish on clean exit, cancel on an exception) with `chunk()` in both `transact` forms, `progress()`, `finish()` and `cancel()`, and the dataclasses `ImportProgress` and `ImportSummary`.

Budgets: the frozen dataclass `QueryBudget(timeout_ms=, reader_timeout_ms=, max_rows=, max_bytes=, cancel_key=)`, `View.with_budget`, `budget=` on `transact` (both forms) and `cypher_write`, `Database(reader_timeout_ms=)`, and `Database.cancel(key)`, which works from another thread because the GIL is released during calls.

Text recall: `Database(text_index=True)`, `View.text_search(text, mode=, graphs=, predicates=, limit=, confidence=)` returning frozen `TextHit` dataclasses with a `TextEvidence`, `Database.rebuild_text_index()` and `Database.enable_text_index()`.

`Database.transact` is both a context manager, which records ops and submits them when the block exits cleanly, and a function of a list of op dicts. The GIL is released during each call, so threads can query one `Database` in parallel.

A `datetime` is written as an `xsd:dateTime` literal (a naive one is taken as UTC) and a `date` as `xsd:date`, and both are read back as the same Python types. `supersede` tells an omitted bound from `None`, which clears it. Wheels and the sdist are built by `.github/workflows/python-wheels.yml`; see `docs/python-publishing.md`.

## Test Strategy

Each binding runs the same story as the time-travel article against a real file, so the two packages and the article cannot drift apart.

The story is: Alice at Acme from 2020, corrected to end in 2024, then at Globex from March 2024, queried as of a transaction, valid at a date, both, and as history. Rust tests of the bridge itself are in `bindings/json/tests/bridge.rs`, and they are the reference the wrapper tests are written against.
