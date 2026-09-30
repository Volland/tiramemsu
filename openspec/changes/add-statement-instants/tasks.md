## Dependencies

- M1 (`add-query-ir-and-sql-planner`, archived) owns `VirtualPred`; M2b (`add-cypher-frontend`, archived) owns the statement time properties. This change extends both and needs no storage change.
- Test code references lat.md test specs with `// @lat: [[tests#…]]`, placed next to the covering test. The requirement text is in `specs/*/spec.md` of this change.

## 1. IR and executor

- [x] 1.1 Add `TM_ADDED_AT` and `TM_RETRACTED_AT` to `tm-ir` `vocab` and to the `VIRTUAL` table, so `is_virtual` recognises them and `Recognised virtual predicates` holds for both.
- [x] 1.2 Add `VirtualPred::AddedAt` and `VirtualPred::RetractedAt`: column `t_add`/`t_ret`, nullable only for `RetractedAt`, object expression `(((SELECT tx.instant FROM tx WHERE tx.t = a.col) << 15) | DATETIME_Z_LOW)`, constant compare `a.col = (SELECT tx.t FROM tx WHERE tx.instant = ?)`, and a constant accepted only when it is a `DATETIME` (compared by instant). Update the module doc table. Unit-test the mapping, the expression and the wrong-kind rule.
- [x] 1.3 Test through the IR (`tm-exec/tests/virtual_preds.rs`): the value under Now, absent `retractedAt` for live statements, the instant under History, the later retraction under `AsOf`, alias reuse (one triple alias), a constant with another offset that seeks `tx_instant`, an instant of no transaction, a wrong-kind constant, and a `FILTER` against a date-time constant.

> Notes (group 1): `object_compare` is now per predicate: the two instants compare `a.t_add`/`a.t_ret` with `(SELECT tx.t FROM tx WHERE tx.instant = ?)`, every other predicate keeps `a.col = ?`. `EXPLAIN QUERY PLAN` of a constant `tm:addedAt` shows the `tx_instant` seek (asserted in `statement_instants`). The alias-reuse path needed no change: the subquery is part of the object expression over the reused alias. `VIRTUAL` grew from 8 to 10 entries; `is_virtual` and the SPARQL lowering pick the new IRIs up by table.

## 2. SPARQL

- [x] 2.1 Confirm the lowering recognises the new IRIs with no code change (virtual predicates are recognised by IRI, and read with no graph selector).
- [x] 2.2 Test the two recipes in SPARQL: "learned late" (`?a > ?f`) and "recorded after it stopped being true" (`?a > ?t`), with a manual clock so instants are fixed, plus the JSON rendering of the value.

> Notes (group 2): no `tm-sparql` change. The test is `learned_late_and_recorded_after_the_fact` in `crates/tiramemsu/tests/sparql_temporal.rs`; it checks the decoded value (`DateTime` with offset `Z`) instead of the JSON text, and adds a constant-instant pattern with a `+02:00` offset. SPARQL has no date-time arithmetic, so the N-day threshold of "learned late" is covered in Cypher only (design.md, Non-Goals).

## 3. Cypher

- [x] 3.1 Replace the repeated `match` on the four temporal names with one table that maps `txAdded`, `txRetracted`, `addedAt`, `retractedAt`, `validFrom` and `validTo` to their IRIs, and use it in property access, pattern property maps and `keys()`.
- [x] 3.2 Decode `tm:addedAt`/`tm:retractedAt` to `DateTime` in `virtual_prop`.
- [x] 3.3 Make `SET` and `REMOVE` of `addedAt`/`retractedAt` fail with `Unsupported` (transaction time is read-only), as for `txAdded`.
- [x] 3.4 Test: values of a live and a retracted relationship, the node form, not in `keys()`, a shadowed stored `v:addedAt`, `SET`/`REMOVE` rejected with nothing written, a property-map match, and the two recipes (`epochMillis` difference over `$days`, and `r.addedAt > r.validTo`).

> Notes (group 3): `access.rs` has `TEMPORAL` (six names), `temporal_iri` and `is_tx_time`; `eval.rs`, `pattern.rs` and `write_set.rs` use them. Fixed a latent bug found by the property-map test: `collect_props` resolved every key of a relationship property map to an IRI, so `-[r {txAdded: 1}]->` (and `validFrom`) never matched the metadata. It now keeps the bare temporal name for relationship maps (`stmt = true`), and `prop_constraint` maps it to the virtual predicate; node maps are unchanged. `CREATE` with an `addedAt` key still writes an ordinary property, as `txAdded` does. Tests: `statement_instants` and `learned_late_and_recorded_after_the_fact` in `crates/tiramemsu/tests/cypher_temporal.rs`.

## 4. Documentation

- [x] 4.1 Update `lat.md/query.md`: the Virtual Predicates table, the Cypher Dual View bullet and the Temporal Syntax statement-time bullet and table row.
- [x] 4.2 Add `lat.md/tests.md` sections for the new tests with `@lat` references, and run `lat check`.

> Notes (group 4): `lat.md/query.md` also gained two bullets under Virtual Predicates (how instants are computed and compared, and the two recipes). New test sections: `tests#Query#Statement Instants Come From The Tx Table`, `Statement Instants Filter By Instant`, `Bitemporal Recipes In SPARQL`, `Cypher Statement Instants`, `Bitemporal Recipes In Cypher`. `lat check` passes.
