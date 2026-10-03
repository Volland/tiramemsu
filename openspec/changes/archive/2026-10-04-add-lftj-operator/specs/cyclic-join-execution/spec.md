## ADDED Requirements

### Requirement: Semantic equivalence
Native cyclic joins SHALL return the same rows and multiplicities as SQL for each supported region, including temporal and graph selections.

#### Scenario: Parallel statements
- **WHEN** parallel eids satisfy one triple pattern
- **THEN** set and bag modes each match their SQL baseline

#### Scenario: Mixed temporal views
- **WHEN** supported patterns select different transaction times
- **THEN** each pattern applies its own view predicates and returns the SQL-equivalent result

### Requirement: Conservative routing
The planner SHALL route to the native operator only when enabled, installed, and applicable. Other shapes SHALL retain SQL routing with an explain reason.

#### Scenario: No installed operator
- **WHEN** LFTJ is enabled but no operator is registered
- **THEN** the region remains SQL and explain identifies the fallback

#### Scenario: Unsupported region
- **WHEN** a region contains unsupported optional algebra
- **THEN** it remains SQL

#### Scenario: Estimate below threshold
- **WHEN** LFTJ is enabled and installed but no pattern of a cyclic region reaches the configured estimate threshold
- **THEN** the region remains SQL and explain identifies the estimate as the reason

#### Scenario: Disabled by default
- **WHEN** a database is opened with default options
- **THEN** no cyclic region is routed natively and the operator is not installed

### Requirement: Measured and bounded execution
Explain SHALL identify native routing, and benchmarks SHALL verify equal results before timing skewed and uniform workloads. The operator SHALL honor operation budgets.

#### Scenario: Triangle benchmark
- **WHEN** SQL and native timing is compared
- **THEN** the harness first verifies identical triangle counts

#### Scenario: Cancellation
- **WHEN** a native join is cancelled
- **THEN** resources are released and no partial successful result is returned
