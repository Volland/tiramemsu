## 1. Plan and commit

- [x] 1.1 Vocabulary `ELPI_DERIVED_BY`, `ELPI_RULE`, `ELPI_SKOLEM`, `ELPI_MATERIALIZED`, `ELPI_RECONCILED` and `ELPI_RECONCILED_COUNT` in `tm-core`.
- [x] 1.2 `tm_elpi_store::plan`: the run with the program's own conclusions excluded, then items with stated support plus the included sources.
- [x] 1.3 Facade `Db::materialize_elpi` through `materialize::commit`, reconciliation, labels and the report.
- [x] 1.4 CLI `tm elpi materialize`.
- [x] 1.5 `@tg: implements::` annotations.

## 2. Tests

- [x] 2.1 Scenario tests on SQLite and redb with `@tg: verifies::`.
