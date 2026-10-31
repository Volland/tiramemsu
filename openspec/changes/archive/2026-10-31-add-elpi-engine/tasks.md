## 1. Engine crate

- [x] 1.1 `crates/tm-elpi`: manifest (`leanelpi` 0.4, `tm-core`, `tm-store`), workspace `[patch.crates-io]` for the local lean-elpi checkout until 0.4 is published.
- [x] 1.2 Prelude, term mapping (`ElpiTerm` ⇄ `Value`) and the monotonicity scan.
- [x] 1.3 Host builtins `tm.triple`, `tm.added`, `tm.retracted`, `tm.valid` over a `ReadStore` and a resolved view, with the seen set and the row budget.
- [x] 1.4 Entry points `query`, `check`, `check_delta` and `conclude`, the support check, interval intersection and the segment sweep for `non_monotonic` programs.
- [x] 1.5 Errors `ElpiProgram`, `ElpiRun` and `ElpiOutOfFuel` in `tm-core`, plus their bridge codes.

## 2. Facade and CLI

- [x] 2.1 Feature `elpi` and `src/elpi.rs`: `View::elpi`, `View::elpi_check` and `View::elpi_conclude` (a preview of conclusions, added while testing), plus the re-exports.
- [x] 2.2 CLI `tm elpi query|check|conclude` behind the CLI feature `elpi` (not a default).
- [x] 2.3 `@tg: implements::` annotations for every requirement.

## 3. Tests

- [x] 3.1 Scenario tests in `crates/tiramemsu/tests/elpi.rs` on SQLite and redb, each with `@tg: verifies::`.
- [x] 3.2 Oracle: a Horn program written in Datalog and in ELPI gives the same conclusions.

## Notes

- lean-elpi's `build.rs` now copies `libleanelpi` into its `OUT_DIR`, so `cargo test` and `cargo run` in dependent crates find it without an rpath.
- The workspace `[patch.crates-io]` for lean-elpi stays until 0.4 is on crates.io.
