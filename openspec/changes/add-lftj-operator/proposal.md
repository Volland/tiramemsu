# Native cyclic-join operator

## Status

Implemented. Opt-in through `OpenOptions::planner.lftj` (`LftjConfig { enabled, min_rows_estimate }`); the operator is `tm_exec::LftjOperator` (re-exported by `tiramemsu`), the table function `tm_lftj(spec)`. Explain reports `RegionKind::NativeLftj` / `RouteNote::LftjNative` or the fallback notes `CyclicLftjDisabled`, `LftjUnavailable`, `LftjUnsupportedShape` and `LftjBelowEstimate`; `View::explain_sparql` explains SPARQL text. JSON bridge: open options `lftj`, `lftjMinRows` and the read `explainSparql`; Node `View.explainSparql`, Python `View.explain_sparql`. Benchmark harness: `crates/tiramemsu/examples/triangles.rs`.

## Why

The triangle sweep reproduces a growing nested-loop penalty on skewed cyclic patterns. The native-operator boundary can host an intersection join without replacing SQLite.

## What Changes

Implement an opt-in native cyclic basic-graph-pattern operator, supported-shape routing, explain notes, and differential benchmarks.

## Capabilities

### New Capabilities

- `cyclic-join-execution`: native cyclic-join operator.

### Modified Capabilities

None in this draft. Implementation must add delta modifications if an existing public requirement must change.

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No new storage backend.
- No acceleration claims for acyclic or unsupported algebra.
- No default routing change until benchmark and semantic gates pass.
