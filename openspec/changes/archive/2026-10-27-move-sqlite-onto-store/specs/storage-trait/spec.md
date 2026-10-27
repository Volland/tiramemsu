## ADDED Requirements

### Requirement: SQLite implements the storage trait
The SQLite hosts SHALL implement `ReadStore` and `WriteStore`, and the facade SHALL run every write on a SQLite file through `tm_store::Engine`, with the file format and the stored rows unchanged.

#### Scenario: Old files keep working
- **WHEN** a database written by 0.8 is opened and written by this version
- **THEN** all its rows are unchanged and new rows follow the same numbering

### Requirement: No regression on SQLite
Moving SQLite onto the storage trait SHALL NOT make any core or SPARQL benchmark more than 5 % slower by median on the same machine.

#### Scenario: Benchmark comparison
- **WHEN** the core and SPARQL benchmarks run before and after the change
- **THEN** no median is more than 5 % slower
