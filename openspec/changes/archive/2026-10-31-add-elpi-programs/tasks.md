## 1. Persistence crate

- [x] 1.1 `crates/tm-elpi-store`: manifest, vocabulary `ELPI_SOURCE`, `ELPI_ENFORCED` (in `tm-core`), error `ProgramCycle`.
- [x] 1.2 `resolve` (includes, cycles, unknown programs) and `info`.
- [x] 1.3 `store_program` and `enforce` over the transaction handle, `RuleLanguageConflict` both ways.
- [x] 1.4 `commit_check`: marks, full check on change, delta check with seeds, the pre-view comparison, the budget.

## 2. Facade and CLI

- [x] 2.1 `TxElpi`, `View::elpi_program_info`, `ElpiProgram::Stored`, the commit hook in `Db::transact_leased` and `with`.
- [x] 2.2 N3 and Datalog stores refuse nodes with `elpi:source`.
- [x] 2.3 CLI `tm elpi program put|show|enforce|unenforce`.
- [x] 2.4 `@tg: implements::` annotations.

## 3. Tests

- [x] 3.1 Scenario tests on SQLite and redb with `@tg: verifies::`.
