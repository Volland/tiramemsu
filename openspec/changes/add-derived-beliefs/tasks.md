## 1. Derive

- [ ] 1.1 Add `SYS_DERIVED_FROM` and make it engine-owned (`crates/tm-core/src/vocab.rs`, `src/engine/reserved.rs`).
- [ ] 1.2 `Tx::derive(s, p, o, valid, support)` in `src/engine/derive.rs`: support validation (non-empty, live, not self), idempotency only for the same live support set, `create` semantics otherwise, schema checks.
- [ ] 1.3 Extend `cascade_set` and `read::dependents` with the support rule in the same breadth-first order (support-derived statements appended with the subject/object dependents of the same parent, ascending eid).
- [ ] 1.4 Supersede: drop derived statements reached through support (not the root) and their structural dependents; replay links when the derived statement is the root.
- [ ] 1.5 Bundles keep `sys:derivedFrom` links; import writes derived statements through `derive`.

## 2. Salience

- [ ] 2.1 `src/salience.rs`: `SaliencePolicy { key, half_life_ms, now }`, `Salience { score, count, last, age }`, one pure function.
- [ ] 2.2 `Tx::increment_volatile(s, key, by)`.
- [ ] 2.3 `View::salience(targets, &policy)` in the facade; `Func::Salience` in the IR, `tm_salience` UDF and SQL join for SPARQL `tm:salience`; Cypher `tiramemsu.salience`.

## 3. Surfaces

- [ ] 3.1 JSON bridge `derive` (transaction op), `incrementVolatile`, `salience` (read); Node and Python wrappers; MCP write tool `derive` (hidden in read-only mode, `additionalProperties: false`).

## 4. Tests

- [ ] 4.1 Retracting a support retracts the derived statement and its layers in the same transaction (kind `cascade`); retracting one of two supports retracts it; a parallel derivation with other support stays live.
- [ ] 4.2 Re-deriving with the same support is `Existing`; deriving over an independently asserted identical statement creates a parallel statement, and retracting the support leaves the independent one live.
- [ ] 4.3 `dependents(e)` includes derived statements; property test: dependents = dry-run retraction = ends of `(^sys:subject|^sys:object|^sys:derivedFrom)*`.
- [ ] 4.4 Supersede of a support drops (does not replay) the derived statement; supersede of the derived statement keeps its support links.
- [ ] 4.5 A saved answer citing a derived statement goes `Stale` with `supportRetracted` after its support is retracted.
- [ ] 4.6 Bundle round trip of a derived belief re-derives it in the target; a second import changes nothing.
- [ ] 4.7 Direct assert of `sys:derivedFrom` fails with `ReservedNamespace`; `max_cascade` counts support-cascaded statements.
- [ ] 4.8 Salience: formula with a fixed `now`; absent counter; as-of view ignores volatile counts; SPARQL, Cypher and Rust give equal scores; nothing is written by a salience read.

## 5. Documentation

- [ ] 5.1 `lat.md/time-model.md#Cascade` (support rule), `lat.md/data-model.md` (derived beliefs), `lat.md/storage.md#Volatile Table` (counters), `lat.md/recipes.md` (summary that goes stale), paper remark and `lat.md/overview.md#Formal Model`; `lat check`; validate OpenSpec.
