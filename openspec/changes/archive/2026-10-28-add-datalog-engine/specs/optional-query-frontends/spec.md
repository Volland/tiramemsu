## ADDED Requirements

### Requirement: Optional Datalog feature
The facade SHALL offer a feature `datalog` that adds `tm-datalog` to the dependency graph and exposes `View::datalog`, `View::datalog_reason`, `View::program_info`, `View::explain_datalog`, `View::check` and `TxDatalog::store_program`. It SHALL be off by default, SHALL NOT enable `exec`, SHALL combine with every other feature combination of the build matrix, and SHALL add no git dependency. Without it `tm-datalog` SHALL NOT appear in `cargo tree -e normal`.

#### Scenario: Default build has no Datalog
- **WHEN** `cargo tree -e normal -p tiramemsu` runs with default features
- **THEN** `tm-datalog` is not listed

#### Scenario: Core tier with Datalog
- **WHEN** `tiramemsu` is built with `--no-default-features --features datalog`
- **THEN** it builds, `View::datalog` works, and `tm-exec` is absent from the dependency tree

#### Scenario: Publishable with Datalog
- **WHEN** `cargo publish --dry-run --no-verify -p tm-datalog` runs (the manifest test also checks that it has no git dependency)
- **THEN** it succeeds
