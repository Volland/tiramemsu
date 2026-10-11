## ADDED Requirements

### Requirement: Statement time in temporal arithmetic
The statement time properties `validFrom`, `validTo`, `addedAt` and `retractedAt` SHALL remain DateTime values with offset `Z` and millisecond precision, and SHALL take part in temporal arithmetic, comparison and `duration.between` like any DateTime. `SET r.validFrom` and `SET r.validTo` SHALL accept a Date (midnight UTC), a LocalDateTime (read as UTC) or a DateTime (its instant), and SHALL fail with `Eval` for a Time, LocalTime or Duration. `txAdded` and `txRetracted` SHALL remain Integers.

#### Scenario: How late a fact was learned
- **WHEN** e1 is valid from 2026-01-01 and was added at 2026-03-10T00:00:00Z, and `MATCH ()-[r]->() RETURN duration.between(r.validFrom, r.addedAt) AS late` runs
- **THEN** `late` for e1 is Duration `P2M9D`

#### Scenario: Recent additions by duration
- **WHEN** `MATCH ()-[r]->() WHERE r.addedAt > datetime.transaction() - duration('P7D') RETURN r` runs
- **THEN** exactly the statements added in the seven days before the query started are returned

#### Scenario: Closing an interval with a local date-time
- **WHEN** e1 has valid interval [2025-01-01, open) and `MATCH ()-[r:worksAt]->() SET r.validTo = localdatetime('2026-03-01T12:00')` runs
- **THEN** e1 is superseded by a statement valid [2025-01-01, 2026-03-01T12:00:00Z)

#### Scenario: Duration is not a valid-time bound
- **WHEN** `MATCH ()-[r:worksAt]->() SET r.validTo = duration('P1D')` runs
- **THEN** it fails with `Eval` and nothing is written
