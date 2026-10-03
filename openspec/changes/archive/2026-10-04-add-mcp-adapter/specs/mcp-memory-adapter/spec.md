## ADDED Requirements

### Requirement: Explicit local configuration
The adapter SHALL open only its configured database and SHALL offer a read-only mode that rejects every write tool before starting a transaction.

#### Scenario: Read-only mutation
- **WHEN** supersede is called in read-only mode
- **THEN** the call fails without modifying graph rows or counters

#### Scenario: Path injection
- **WHEN** a tool request includes an alternate database path
- **THEN** the request is rejected and the configured file remains the only target

### Requirement: Typed existing semantics
Tools SHALL use existing assertion, confirmation, supersede, dependents, and bundle semantics with structured term validation and stable errors.

#### Scenario: Repeated assertion
- **WHEN** the same overlapping-valid-time fact is asserted twice
- **THEN** the second call identifies the existing statement

#### Scenario: Invalid bundle
- **WHEN** import_bundle receives a malformed bundle
- **THEN** the tool returns a structured argument error and commits nothing

### Requirement: Bounded auditable query results
Query tools SHALL apply configured budgets and include the selected view and provenance coverage. Unsupported query forms SHALL be reported as errors rather than retried as another operation.

#### Scenario: Incomplete provenance
- **WHEN** a recursive-path query returns endpoints without supporting statement ids
- **THEN** the response marks provenance coverage as incomplete

#### Scenario: Limit exceeded
- **WHEN** a query exceeds its execution or result budget
- **THEN** the response is a structured error and the next request can run
