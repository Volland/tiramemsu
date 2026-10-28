## ADDED Requirements

### Requirement: Registry dependency without network clients
The workspace SHALL depend on eyeron from crates.io with default features off, and `cargo tree -e normal -p tm-n3` SHALL NOT contain an HTTP client.

#### Scenario: No HTTP client
- **WHEN** `cargo tree -e normal -p tm-n3` runs
- **THEN** neither `ureq` nor any other HTTP client is listed

### Requirement: Clock from options
`tm-n3` SHALL pass the run's `now` to eyeron through `ReasonerOptions::now` and SHALL NOT rewrite `time:localTime` premises.

#### Scenario: Pinned clock without rewrite
- **WHEN** a rule using `time:localTime` runs on an as-of view
- **THEN** it yields the view's instant and the rules eyeron receives are the stored text unchanged
