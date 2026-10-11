# Interval algebra over valid time

Target release: **0.7 — Time in every language**. Source: `docs/feature-roadmap.md#0.7` ("Interval algebra").

## Why

Every statement has a half-open valid interval `[v_from, v_to)` with NULL meaning unbounded (`lat.md/time-model.md#Valid Time`), and the questions agents ask about it are interval questions: which beliefs overlapped this period, which job ended exactly when the next began, which facts held throughout a project. Today each is four NULL-aware comparisons in a `FILTER`, written by hand over `tm:validFrom`/`tm:validTo` in SPARQL or `r.validFrom`/`r.validTo` in Cypher, and easy to get wrong at the open ends. The engine already has the right test in one place, the overlap test of assert (`lat.md/time-model.md#Operations#Assert`), but no query can name it.

Allen's thirteen relations are the standard vocabulary for this, and half-open intervals make them exact: `meets` is "ends where the other starts", with no shared instant and no gap.

## What Changes

- **A shared interval model** in `tm-core`: the existing `Valid { from, to }` (`crates/tm-core/src/report.rs`) gains `relate(&self, &Valid) -> Allen` returning exactly one of the thirteen relations, with unbounded ends treated as −∞ and +∞ (two unbounded ends of the same side are equal), and `intersects`, the overlap test assert already uses.
- **SPARQL functions** in the `tm:` namespace: `tm:before`, `tm:after`, `tm:meets`, `tm:metBy`, `tm:overlaps`, `tm:overlappedBy`, `tm:starts`, `tm:startedBy`, `tm:during`, `tm:contains`, `tm:finishes`, `tm:finishedBy`, `tm:equals` (booleans), `tm:intersects` (any shared instant), `tm:allen(a, b)` (the relation's name), `tm:interval(from, to)` and `tm:validInterval(?r)` (interval values as `tm:Interval` literals). An operand is a statement (its valid interval), an interval value, or an `xsd:date`/`xsd:dateTime` instant (a one-millisecond interval). `tm:unbounded` stands for an open end in `tm:interval`.
- **Cypher functions** with the same semantics: `tiramemsu.interval.before(a, b)` … `tiramemsu.interval.equals(a, b)`, `tiramemsu.interval.intersects`, `tiramemsu.interval.relation(a, b)`, and `tiramemsu.interval(from, to)` returning a map `{from, to}`. An operand is a relationship or statement node, such a map, or a Date, DateTime or LocalDateTime instant.
- **Lowering:** SPARQL functions lower to new IR functions (`Func::Allen(rel)`, `Func::Intersects`, `Func::AllenName`, `Func::Interval`, `Func::ValidInterval`) that SQL codegen expands into NULL-aware comparisons on the operand statements' `v_from`/`v_to`, so they need no UDF on the SQL path and can use `valid_p` for an `intersects` with a constant interval. Cypher evaluates them in Rust with the same `Valid::relate`.
- **Differential tests:** each relation is checked against a brute-force oracle over all endpoint orderings (including unbounded ends), and SPARQL and Cypher answer the same questions with the same statements.

## Capabilities

### New Capabilities

- `interval-algebra`: Allen relations, intersection and interval values over valid time, in Rust, SPARQL and Cypher.

### Modified Capabilities

- `sparql-query`: "Built-in functions" accepts the `tm:` interval functions as the one exception to "any custom function IRI is Unsupported".

## Impact

- **`crates/tm-core`:** `src/report.rs` (`Valid::relate`, `Valid::intersects`, enum `Allen`), `src/vocab.rs` (`TM_INTERVAL` datatype, function IRIs, `TM_UNBOUNDED`).
- **`crates/tm-ir`:** `src/expr.rs` (`Func` variants), `src/display.rs`, `src/validate.rs`.
- **`crates/tm-exec`:** `src/sqlgen/expr.rs` (expansion to SQL over `v_from`/`v_to` of the operand statement, joining `triple` by eid when the operand is not the eid of a pattern in scope), `src/decode.rs` (rendering `tm:Interval` literals), `src/udf.rs` (comparison class of interval literals).
- **`crates/tm-sparql`:** `src/lower/expr.rs` (`Function::Custom` for `tm:` interval IRIs before the `Unsupported` fallback).
- **`crates/tm-cypher`:** `src/exec/funcs.rs`, `src/funcs.rs` (registry), `src/sema/check.rs` (arity).
- **Tests:** `crates/tm-core` unit and property tests, `crates/tm-sparql/tests`, `crates/tm-cypher/tests/golden_ir.rs`-style tests, the dialect differential suite in `crates/tiramemsu/tests`.
- **Docs:** `lat.md/time-model.md#Valid Time`, `lat.md/query.md#Temporal Syntax`, `lat.md/recipes.md` (overlap recipe rewritten with `tm:intersects`).
- **Not affected:** storage format, write semantics, transaction time. Depends on nothing; can land before or after `add-cypher-temporal-types` (operands that are Time, LocalTime or Duration fail with a type error either way).

## Non-goals

- No interval algebra over transaction time (`[t_add, t_ret)`) in this change; the model admits it, but the query syntax for choosing the axis is left open.
- No interval-valued storage; intervals stay columns of statements.
- No temporal joins beyond what a `FILTER` with these functions expresses.
