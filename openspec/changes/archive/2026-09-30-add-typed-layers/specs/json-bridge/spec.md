## MODIFIED Requirements

### Requirement: Errors carry a code

Every failure SHALL be `{"code", "message"}`, where `code` is the name of the core error variant (for example `Parse`, `Unsupported`, `NotLive`, `UniqueViolation`, `SubjectTypeMismatch`) or `InvalidArgument` when the bridge rejected the call itself. `call_text` SHALL return the failure as that JSON text.

#### Scenario: Parse error
- **WHEN** `sparql` runs the text `SELECT ?`
- **THEN** it fails with code `Parse`, and `call_text` returns text containing `"code":"Parse"`

#### Scenario: Write through a read-only view
- **WHEN** `cypher` runs `CREATE (:Person)`
- **THEN** it fails with code `Unsupported`

#### Scenario: Missing argument
- **WHEN** an `assert` op has no `p` and no `o`
- **THEN** it fails with code `InvalidArgument`

#### Scenario: Subject type violation
- **WHEN** `v:confidence` has `sys:subjectType sys:STMT` and a `transact` call asserts `(v:alice v:confidence 0.8)`
- **THEN** it fails with code `SubjectTypeMismatch`, and nothing from that call is committed
