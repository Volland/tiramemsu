# optional-query-frontends Specification

## Purpose
TBD - created by archiving change add-optional-query-frontends. Update Purpose after archive.

## Requirements

### Requirement: Core-only dependency graph
A no-default-features core-only facade build SHALL omit the query IR, the executor and the SPARQL and Cypher parser dependencies while retaining transaction, view, text recall, conflict review, bulk import, budget and bundle-JSON functionality. Bundle serialisation (JSON and N-Triples) SHALL NOT require a query parser.

#### Scenario: Dependency audit
- **WHEN** the core-only normal dependency tree is inspected
- **THEN** neither query frontend parser (`spargebra`, `peg`, `open-cypher`) nor `tm-ir`, `tm-exec`, `tm-sparql` or `tm-cypher` is present

#### Scenario: Core operations
- **WHEN** a core-only binary asserts and reads a fact
- **THEN** it works with the same persisted format and temporal semantics

#### Scenario: Bundle formats
- **WHEN** a core-only build exports a fact bundle as JSON and N-Triples and imports the JSON into another database
- **THEN** the bundle round-trips exactly without a parser

#### Scenario: One file across builds
- **WHEN** a database file written by a core-only build is opened by a default build, and the reverse
- **THEN** both builds see the same statements in every view

### Requirement: Compatible default surface
Default features SHALL retain existing SPARQL, Cypher, paths, saved answers and bundle serialization behavior. Disabled frontend APIs SHALL be absent with documented feature requirements, and the change of meaning of the existing `sparql` feature SHALL be documented with a migration.

#### Scenario: Default application
- **WHEN** an existing default-feature example is built
- **THEN** it continues to compile and run

#### Scenario: Single frontend
- **WHEN** only SPARQL is enabled
- **THEN** SPARQL and shared execution work without linking the Cypher parser

#### Scenario: Cypher only
- **WHEN** only Cypher is enabled
- **THEN** Cypher reads, writes and `TxCypher` work without linking the SPARQL parser

#### Scenario: Execution only
- **WHEN** only the shared execution feature is enabled
- **THEN** paths and IR execution work without linking either parser

### Requirement: Build matrix
CI SHALL verify core-only, execution-only, individual frontends, and default features, including binding configurations and documentation.

#### Scenario: Feature matrix
- **WHEN** feature combinations are checked
- **THEN** dependency and runtime smoke checks pass for each supported combination

#### Scenario: Binding configurations
- **WHEN** the JSON bridge, the MCP server and the Node and Python bindings are inspected
- **THEN** their dependency trees contain both query frontends

### Requirement: Optional N3 reasoning feature
The facade SHALL offer a feature `n3` that adds `tm-n3` and `eyeron` to the dependency graph and exposes `View::reason`, `View::rule_set_info`, `TxN3::store_rules` and `Db::materialize`. It SHALL be off by default, SHALL NOT enable `exec`, and SHALL combine with every other feature combination of the build matrix. Without it neither `tm-n3` nor `eyeron` SHALL appear in `cargo tree -e normal`.

#### Scenario: Default build has no reasoner
- **WHEN** `cargo tree -e normal -p tiramemsu` runs with default features
- **THEN** neither `tm-n3` nor `eyeron` is listed

#### Scenario: Core tier with reasoning
- **WHEN** `tiramemsu` is built with `--no-default-features --features n3`
- **THEN** it builds, `View::reason` works, and `tm-exec` is absent from the dependency tree

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
