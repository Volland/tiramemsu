## ADDED Requirements

### Requirement: Optional N3 reasoning feature
The facade SHALL offer a feature `n3` that adds `tm-n3` and `eyeron` to the dependency graph and exposes `View::reason`, `View::rule_set_info`, `TxN3::store_rules` and `Db::materialize`. It SHALL be off by default, SHALL NOT enable `exec`, and SHALL combine with every other feature combination of the build matrix. Without it neither `tm-n3` nor `eyeron` SHALL appear in `cargo tree -e normal`.

#### Scenario: Default build has no reasoner
- **WHEN** `cargo tree -e normal -p tiramemsu` runs with default features
- **THEN** neither `tm-n3` nor `eyeron` is listed

#### Scenario: Core tier with reasoning
- **WHEN** `tiramemsu` is built with `--no-default-features --features n3`
- **THEN** it builds, `View::reason` works, and `tm-exec` is absent from the dependency tree
