## Context

The facade (`Db`, `View`, `Tx`) is synchronous and `Send + Sync`. Oxilite's bindings show the cheapest way to expose a Rust store to two languages: keep the native layer trivially small and move all conversion to one place. Its Node and Python crates exchange JSON strings with a wrapper written in the host language. Only the wrappers differ.

## Goals / Non-Goals

**Goals:**
- Both packages expose the same operations with the same results, so a fix or a new feature lands once.
- Both clocks are first-class: every read takes a view with `asOf` and `validAt`.
- A transaction can write a layer on a statement it just created, in one call.
- Errors carry a stable code that a caller can branch on.
- Both packages install from a registry without a Rust toolchain.

**Non-Goals:** WASM, MCP, async, streaming.

## Decisions

**D1. One bridge crate, `Database::call(op, args)`.** Rather than one napi function and one pyo3 function per facade method, the native crates export a class with a constructor and a single `call`. A new operation is one match arm in the bridge, invisible to both native crates. The cost is a JSON parse and print per call, which is small next to a SQLite query; the alternative (typed napi and pyo3 signatures) would double every change. Oxilite made the same choice.

**D2. Terms are plain JSON where JSON is exact, tagged objects otherwise.** A string is a plain string, a number an integer or double, a boolean a boolean; `{"iri"}`, `{"node"}`, `{"bnode"}`, `{"stmt"}`, `{"tx"}`, `{"$int"}` and `{"lex", "datatype" | "lang"}` cover the rest. Values that JSON cannot carry exactly (integers beyond 2^53, whole doubles, dates, decimals) leave as `{"lex", "datatype"}` and are accepted back unchanged, so a result can be written straight back. `{"$int"}` matches the form `CypherValue::to_json` already uses.

**D3. Transactions are lists of op objects with named references.** A layer needs the id of the statement it is about, which does not exist until the transaction runs. An op carries `"as": name`; a later op uses `{"ref": name}` as a statement id, or as a subject or object. The Python context manager and the TypeScript callback both record ops and submit them once, so a block that throws submits nothing.

**D4. A view is data.** `{"kind", "tx", "instant", "validAt"}` is the same object for every read. Valid-time filtering is off unless `validAt` is present, matching `lat.md/query#Temporal Syntax`.

**D5. Errors are the variant name plus a message.** The bridge maps each core `Error` variant to a code (`Parse`, `NotLive`, `UniqueViolation`, …) and adds `InvalidArgument` for a call it rejects itself. An argument error inside a transaction body travels as `Error::Custom` and is unwrapped at the boundary, so the whole transaction fails and nothing commits. The native crates prefix the JSON error text with `tiramemsu:` so a wrapper can tell it from a host error.

**D6. The binding crates opt out of the workspace lint.** `unsafe_code = "forbid"` cannot be overridden by `#[allow]`, and napi and pyo3 macros generate `unsafe`. The three crates simply do not set `[lints] workspace = true`. The facade and core crates keep `forbid`.

**D7. Python on the stable ABI, Node with one addon per platform.** `abi3-py39` gives one wheel per platform for every Python from 3.9. Node addons are not ABI-stable across all uses, so the npm package carries one prebuilt `.node` file per platform, chosen at load time.

**D8. Trusted publishing, no stored tokens.** PyPI publishes through OIDC from a GitHub environment (`pypi`); npm publishes with provenance from an environment (`npm`) that holds one automation token as a secret. Publishing needs a release tag or a manual run with `publish` checked.

## Risks / Trade-offs

- **JSON overhead on large result sets.** A `SELECT` of a million rows is one big string. Acceptable for agent memory; streaming is a stated non-goal.
- **Two wrappers can drift.** Mitigated by the bridge doing all conversion and by both test suites running the same story as the time-travel article.
- **Whole doubles read back as `{"lex", "datatype"}`,** which surprises a caller who expected `3.0`. The wrappers convert it back to the language's float.
- **The npm package is only as portable as its prebuilt files.** A platform without a prebuilt addon fails at import with a message naming the platform.

## Open Questions

None blocking. The Windows and musl Node addons are built in CI; whether to publish them depends on their smoke tests passing there.
