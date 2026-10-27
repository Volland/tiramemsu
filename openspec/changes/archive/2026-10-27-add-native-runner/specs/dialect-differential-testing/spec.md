## ADDED Requirements

### Requirement: Differential suite on both runners
The differential suite SHALL also run every query pair on a redb database through the native runner and SHALL require the same normalised results as on SQLite.

#### Scenario: Corpus on redb
- **WHEN** the suite runs with the redb backend enabled
- **THEN** every query pair produces the same normalised results on both backends
