## ADDED Requirements

### Requirement: Subject type constrains subjects
For a predicate with one or more live `sys:subjectType` statements, an assert or create whose subject's tag is not the tag named by one of them SHALL fail with `SubjectTypeMismatch { p, expected, got }`, where `p` is the predicate, `expected` lists the live tag IRIs in eid order, and `got` is the subject's tag; the transaction SHALL leave no trace. The error message SHALL name the predicate, the allowed tag IRIs and the rejected tag. Several live values SHALL mean any-of. A predicate without the flag SHALL accept any subject kind. The check SHALL apply to every user write path that asserts or creates a statement, including SPARQL `INSERT` and Cypher `CREATE`, `SET` and `MERGE`. Supersede keeps the subject and SHALL NOT repeat the check.

#### Scenario: Typed layer accepts a statement subject
- **WHEN** `(:confidence sys:subjectType sys:STMT)` is live, e1 is a live statement, and `(e1 :confidence 0.8)` is asserted
- **THEN** the statement is inserted

#### Scenario: Typed layer rejects a node subject
- **WHEN** `(:confidence sys:subjectType sys:STMT)` is live and `(alice :confidence 0.8)` is asserted
- **THEN** the transaction fails with `SubjectTypeMismatch` naming `:confidence`, `sys:STMT` and `IRI`
- **AND** no statement from that transaction exists

#### Scenario: Several values mean any-of
- **WHEN** `(:note sys:subjectType sys:STMT)` and `(:note sys:subjectType sys:TX)` are live
- **THEN** `(e1 :note "x")` and `(tx5 :note "y")` are inserted, and `(alice :note "z")` fails with `SubjectTypeMismatch` whose `expected` lists both tag IRIs

#### Scenario: Rejected through SPARQL
- **WHEN** `(v:confidence sys:subjectType sys:STMT)` is live and `INSERT DATA { v:alice v:confidence 0.8 }` is submitted
- **THEN** the request fails with `SubjectTypeMismatch` and nothing is written, while `INSERT DATA { v:alice v:worksAt v:acme ~ _:r {| v:confidence 0.8 |} }` succeeds

#### Scenario: Rejected through Cypher
- **WHEN** `(v:confidence sys:subjectType sys:STMT)` is live and `CREATE (:Person {confidence: 0.8})` runs
- **THEN** it fails with `SubjectTypeMismatch` and nothing is written, while `MATCH ()-[r:worksAt]->() SET r.confidence = 0.8` succeeds

## MODIFIED Requirements

### Requirement: Schema flags are versioned statements
A predicate's schema SHALL be expressed by live statements whose subject is the predicate's IRI and whose predicate is `sys:cardinality`, `sys:unique`, `sys:valueType`, `sys:subjectType` or `sys:isEdge`. These statements SHALL be ordinary statements with eids, transaction times and history. A flag SHALL be in force exactly while its statement is live in transaction time; its valid interval SHALL NOT affect enforcement. A predicate without flags SHALL behave as cardinality many with no constraint. Enforcement SHALL always use the flags live at the current point of the writing transaction.

#### Scenario: Default behaviour
- **WHEN** `(alice :likes tea)` and `(alice :likes coffee)` are asserted on a predicate without flags
- **THEN** both are live

#### Scenario: Schema history
- **WHEN** `(:age sys:cardinality sys:one)` is asserted in transaction 10
- **THEN** the as-of view at transaction 9 does not contain it and the now view does

#### Scenario: Flag takes effect within its transaction
- **WHEN** one transaction asserts `(:email sys:unique true)` and then asserts `(alice :email "a@x.org")` and `(bob :email "a@x.org")`
- **THEN** the transaction fails with `UniqueViolation`

#### Scenario: Retracting a flag lifts the constraint
- **WHEN** the `sys:unique` flag of `:email` is retracted
- **THEN** a later transaction may assert the same email for two subjects

#### Scenario: Subject type takes effect within its transaction
- **WHEN** one transaction asserts `(:confidence sys:subjectType sys:STMT)` and then `(alice :confidence 0.8)`
- **THEN** the transaction fails with `SubjectTypeMismatch`

### Requirement: Schema flag values are validated
The subject of a schema flag SHALL be an `IRI` outside the `sys:` namespace, otherwise the write SHALL fail with `ReservedNamespace` (for a `sys:` IRI) or an invalid-term error (for any other kind). The object SHALL be `sys:one` or `sys:many` for `sys:cardinality`; a `BOOL` for `sys:unique` and `sys:isEdge`; for `sys:valueType` either a tag IRI (`sys:` followed by a tag name, for example `sys:INT` or `sys:STMT`) or a datatype IRI; and for `sys:subjectType` the tag IRI of a subject kind: `sys:IRI`, `sys:NODE`, `sys:BNODE`, `sys:STMT` or `sys:TX`. Any other object SHALL fail with `ValueTypeMismatch { p, expected, got }` where `p` is the flag predicate. Each flag except `sys:subjectType` SHALL hold at most one live value per predicate: asserting a new value SHALL retract the previous one with kind `cardinality`. `sys:subjectType` MAY hold several live values; asserting another value SHALL add it and retract nothing, and re-asserting a live value SHALL return the existing statement.

#### Scenario: Invalid cardinality value
- **WHEN** `(:age sys:cardinality "one")` is asserted
- **THEN** the transaction fails with `ValueTypeMismatch` for `sys:cardinality`

#### Scenario: Flag on a reserved predicate
- **WHEN** `(sys:reason sys:cardinality sys:one)` is asserted
- **THEN** the transaction fails with `ReservedNamespace`

#### Scenario: Changing a flag value
- **WHEN** `(:tag sys:cardinality sys:one)` is live and `(:tag sys:cardinality sys:many)` is asserted
- **THEN** the first flag statement is retracted with kind `cardinality` and only `sys:many` is live

#### Scenario: Unique set to false
- **WHEN** `(:email sys:unique false)` is live
- **THEN** `:email` behaves as a predicate without a uniqueness constraint

#### Scenario: Subject type that no subject can have
- **WHEN** `(:confidence sys:subjectType sys:INT)`, `(:confidence sys:subjectType xsd:integer)` or `(:confidence sys:subjectType "STMT")` is asserted
- **THEN** the transaction fails with `ValueTypeMismatch` for `sys:subjectType`

#### Scenario: A second subject type is added
- **WHEN** `(:note sys:subjectType sys:STMT)` is live and `(:note sys:subjectType sys:TX)` is asserted
- **THEN** both flag statements are live and nothing is retracted

### Requirement: Schema changes violated by live data are rejected
Asserting a schema flag that live data already violates SHALL fail with `SchemaConflict { violating }` listing, in ascending order, the eids of every live statement involved in a violation, and the transaction SHALL leave no trace. For `sys:cardinality sys:one`, the violating eids are all live statements of the predicate that share a subject with another live statement of a different object and overlapping valid time. For `sys:unique true`, they are all live statements whose object is held by more than one subject. For `sys:valueType`, they are all live statements whose object does not match. For `sys:subjectType`, they are all live statements of the predicate whose subject's tag is outside the set of subject types the predicate would have after the change: the live values plus the new one for an assert, the live values without the superseded statement plus the new value for a supersede, and the live values without the retracted statement for a retraction. Retracting the last `sys:subjectType` value lifts the constraint and SHALL NOT conflict. Asserting `sys:cardinality sys:many`, `sys:unique false` or `sys:isEdge`, or retracting any other flag, SHALL never conflict. The check SHALL see earlier writes of the same transaction.

#### Scenario: Cardinality one over conflicting data
- **WHEN** `(alice :age 30)` and `(alice :age 31)` are live with overlapping valid time and `(:age sys:cardinality sys:one)` is asserted
- **THEN** the transaction fails with `SchemaConflict` listing both eids

#### Scenario: Cardinality one over disjoint episodes
- **WHEN** `(alice :worksAt acme)` valid `[2020, 2022)` and `(alice :worksAt globex)` valid `[2022, unbounded)` are live, and `(:worksAt sys:cardinality sys:one)` is asserted
- **THEN** the flag is inserted

#### Scenario: Unique over duplicate values
- **WHEN** `(alice :email "a@x.org")` and `(bob :email "a@x.org")` are live and `(:email sys:unique true)` is asserted
- **THEN** the transaction fails with `SchemaConflict` listing both eids

#### Scenario: Value type over mismatching data
- **WHEN** `(alice :age 30)` and `(bob :age "old")` are live and `(:age sys:valueType xsd:integer)` is asserted
- **THEN** the transaction fails with `SchemaConflict` listing only Bob's statement

#### Scenario: Conflict created earlier in the same transaction
- **WHEN** one transaction asserts `(alice :email "a@x.org")` and `(bob :email "a@x.org")` and then `(:email sys:unique true)`
- **THEN** the transaction fails with `SchemaConflict`

#### Scenario: Subject type over node-level data
- **WHEN** `(e1 :confidence 0.8)` and `(alice :confidence 0.5)` are live and `(:confidence sys:subjectType sys:STMT)` is asserted
- **THEN** the transaction fails with `SchemaConflict` listing only the `alice` statement

#### Scenario: Retracting one of several subject types narrows the set
- **WHEN** `(:note sys:subjectType sys:STMT)` and `(:note sys:subjectType sys:TX)` are live, `(tx5 :note "y")` is live, and the `sys:TX` flag statement is retracted
- **THEN** the transaction fails with `SchemaConflict` listing the `tx5` statement

#### Scenario: Retracting the last subject type lifts the constraint
- **WHEN** `(:confidence sys:subjectType sys:STMT)` is the only value and it is retracted
- **THEN** the retraction succeeds and `(alice :confidence 0.8)` may then be asserted

### Requirement: Order of schema checks
For every inserted statement the checks SHALL run in the order value type, subject type, then uniqueness, then cardinality-one replacement, and a failing check SHALL prevent all later ones. For assert, the value-type and subject-type checks SHALL run before the idempotency lookup, and the uniqueness and cardinality steps SHALL run only when a new statement is to be inserted.

#### Scenario: Failed value type prevents cardinality replacement
- **WHEN** `:age` is `sys:one` with `sys:valueType xsd:integer`, `(alice :age 30)` is live, and `(alice :age "thirty")` is asserted
- **THEN** the error is `ValueTypeMismatch`
- **AND** `(alice :age 30)` is still live

#### Scenario: Uniqueness checked before cardinality replacement
- **WHEN** `:email` is unique and `sys:one`, `(alice :email "a@x.org")` and `(bob :email "b@x.org")` are live, and `(bob :email "a@x.org")` is asserted
- **THEN** the error is `UniqueViolation`
- **AND** `(bob :email "b@x.org")` is still live

#### Scenario: Failed subject type prevents cardinality replacement
- **WHEN** `:score` is `sys:one` with `sys:subjectType sys:STMT`, `(e1 :score 1)` is live, and `(alice :score 2)` is asserted
- **THEN** the error is `SubjectTypeMismatch`
- **AND** `(e1 :score 1)` is still live
