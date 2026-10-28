# Read-time salience

Target release: **0.6 — Agent recall**. Split on 2026-10-28 from `add-derived-beliefs`, which now covers derivation only.

## Why

Agents need to rank what to bring back. The store holds access patterns only as volatile values (`volatile(s, key, value, updated_at)`, `crates/tm-core/src/engine/volatile.rs`) and recency as `tm:addedAt`, but has no way to combine them into a score, and the never-forget rule means forgetting cannot be the answer: low-salience memory should rank lower, not disappear.

## What Changes

- **Read-time salience.** A deterministic score `salience = (1 + ln(1 + count)) · 2^(−age / half_life)` per node or statement, where `count` is an integer volatile counter under a chosen key, and `age` is `now` minus the latest of the counter's `updated_at` and the newest `tm:addedAt` of the target's live statements. It is exposed as `View::salience(targets, &SaliencePolicy)`, SPARQL `tm:salience(?x, <counterKey>, "P7D"^^xsd:dayTimeDuration)`, Cypher `tiramemsu.salience(x, {key, halfLife})`, and the bridge op `salience`, usable in `ORDER BY` and `FILTER`. `Tx::increment_volatile(s, key, by)` records an access atomically. Salience never writes and never retracts.

## Capabilities

### New Capabilities

- `read-time-salience`: the salience score, its policy, query functions and the volatile counter increment.

## Impact

- **`crates/tm-core`, `crates/tm-store`:** `engine/volatile.rs` (`increment_volatile` in both engines), new `src/salience.rs` (score and policy, pure function shared by all surfaces).
- **`crates/tm-exec`:** `src/udf.rs` (`tm_salience` scalar UDF), `src/sqlgen/expr.rs`, `crates/tm-ir/src/expr.rs` (`Func::Salience`), the native runner's expression evaluator.
- **`crates/tm-sparql`:** `src/lower/expr.rs` (`tm:salience`); **`crates/tm-cypher`:** `src/exec/funcs.rs`, `src/funcs.rs`.
- **Facade, bindings:** `View::salience`, bridge `incrementVolatile` and `salience`.

## Non-goals

- No automatic decay that retracts or hides anything.
