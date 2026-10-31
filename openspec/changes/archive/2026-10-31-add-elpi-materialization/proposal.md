# Materialize ELPI conclusions

Source: decisions E3, E4, E7 and E9 of `lat.md/elpi.md` (interview of 2026-10-31). Change 3 of four; depends on `add-elpi-programs`.

## Why

Conclusions that are stored become ordinary statements: SPARQL, Cypher and Datalog read them, and they fall when their premises fall. N3 and Datalog already share the commit half of materialization (`materialize.rs`). ELPI should use the same path, so all three languages derive beliefs in one way.

## What Changes

- **`Db::materialize_elpi(program, opts)`:** runs `conclude` on a snapshot outside the writer, then writes every conclusion through the shared commit, using `derive`, in one short transaction. A conclusion's support is the eids it states, plus the program's source statement, plus the source statements of every program it includes.
- **Always reconcile:** live conclusions of the program that this run did not derive again are retracted. The transaction is labelled `(tx elpi:materialized program)`, and `elpi:reconciled` and `elpi:reconciledCount` are added when something was retracted. A `non_monotonic` program is listed in `needs_rerun_after_writes`.
- **Own conclusions are not input:** the program's live conclusions, and the statements about them, are hidden from its own run.
- **Plan in `tm-elpi-store`:** `plan(store, spec, program, opts)` returns the items in the shape the shared commit takes.
- **CLI:** `tm elpi materialize`.

## Capabilities

### New Capabilities

- `elpi-materialization`.

## Impact

- `tm-elpi-store` (`plan`), the facade's `src/elpi_materialize.rs`, the `tm-core` vocabulary `ELPI_DERIVED_BY`, `ELPI_RULE`, `ELPI_SKOLEM`, `ELPI_MATERIALIZED`, `ELPI_RECONCILED` and `ELPI_RECONCILED_COUNT`.
- lat.md: `lat.md/elpi.md` (E3, E4 implemented).
