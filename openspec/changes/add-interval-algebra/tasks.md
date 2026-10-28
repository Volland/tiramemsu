## 1. Interval model

- [x] 1.1 Add `Allen` (thirteen variants with stable names) and `Valid::relate`, `Valid::intersects` in `crates/tm-core/src/report.rs`; reuse `Valid::intersects` in the assert overlap check so there is one definition.
- [x] 1.2 Property test: for random proper intervals with optional infinite ends, exactly one relation holds, converses are symmetric, and `intersects` equals the disjunction of the nine intersecting relations and the assert overlap SQL.

## 2. SPARQL

- [ ] 2.1 Add `TM_INTERVAL`, `TM_UNBOUNDED` and the function IRIs to `crates/tm-core/src/vocab.rs`.
- [ ] 2.2 IR: `Func::Allen(Allen)`, `Func::Intersects`, `Func::AllenName`, `Func::Interval`, `Func::ValidInterval` in `crates/tm-ir/src/expr.rs`, with display and validation (arity, operand kinds known at plan time).
- [ ] 2.3 Lower `tm:` interval IRIs in `crates/tm-sparql/src/lower/expr.rs` before the `Unsupported` fallback.
- [ ] 2.4 SQL expansion in `crates/tm-exec/src/sqlgen/expr.rs`: direct alias columns for pattern eids, correlated lookup otherwise, sentinels for infinities, `valid_p`-friendly form for constant `intersects`; decode and order `tm:Interval` literals.

## 3. Cypher

- [ ] 3.1 Register `tiramemsu.interval.*` and `tiramemsu.interval` in `crates/tm-cypher/src/funcs.rs`; evaluate with `Valid::relate` in `src/exec/funcs.rs`; operand conversion for relationships, statement nodes, `{from, to}` maps and instants.

## 4. Tests

- [ ] 4.1 SPARQL: each relation on fixtures covering bounded, half-open and always-valid statements; instants; `tm:interval` with `tm:unbounded`; error cases (non-statement operand, inverted interval) are false in `FILTER` and unbound in `BIND`.
- [ ] 4.2 Cypher: the same fixtures and errors (type error is `Eval`).
- [ ] 4.3 Differential: "which beliefs intersect [2025-01-01, 2025-07-01)", "which jobs meet the next job", "facts during a project" return the same statements in both languages and through Rust.
- [ ] 4.4 Plan test: a constant `tm:intersects` on a live predicate pattern uses `valid_p`.

## 5. Documentation

- [ ] 5.1 `lat.md/time-model.md#Valid Time`, `lat.md/query.md#Temporal Syntax`, a recipe in `lat.md/recipes.md` with its test; `lat check`; validate OpenSpec.
