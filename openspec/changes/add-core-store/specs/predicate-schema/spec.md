## Purpose

Defines the optional per-predicate schema stored as ordinary `sys:` statements (cardinality, uniqueness with upsert, value type and the edge flag), how each flag constrains writes inside the writer transaction, and how schema changes that live data already violates are rejected.

## ADDED Requirements

### Requirement: Schema flags are versioned statements
A predicate's schema SHALL be expressed by live statements whose subject is the predicate's IRI and whose predicate is `sys:cardinality`, `sys:unique`, `sys:valueType` or `sys:isEdge`. These statements SHALL be ordinary statements with eids, transaction times and history. A flag SHALL be in force exactly while its statement is live in transaction time; its valid interval SHALL NOT affect enforcement. A predicate without flags SHALL behave as cardinality many with no constraint. Enforcement SHALL always use the flags live at the current point of the writing transaction.

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

### Requirement: Schema flag values are validated
The subject of a schema flag SHALL be an `IRI` outside the `sys:` namespace, otherwise the write SHALL fail with `ReservedNamespace` (for a `sys:` IRI) or an invalid-term error (for any other kind). The object SHALL be `sys:one` or `sys:many` for `sys:cardinality`; a `BOOL` for `sys:unique` and `sys:isEdge`; and for `sys:valueType` either a tag IRI (`sys:` followed by a tag name, for example `sys:INT` or `sys:STMT`) or a datatype IRI. Any other object SHALL fail with `ValueTypeMismatch { p, expected, got }` where `p` is the flag predicate. Each flag SHALL hold at most one live value per predicate: asserting a new value SHALL retract the previous one with kind `cardinality`.

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

### Requirement: Cardinality one replaces overlapping objects
For a predicate flagged `sys:cardinality sys:one`, an assert or create of `(s, p, o)` that inserts a new statement SHALL first retract every live statement `(s, p, o')` with `o' ≠ o` whose valid interval overlaps the new one, with kind `cardinality` and with cascade, and SHALL NOT replay any annotation of the retracted statements. Live statements `(s, p, o')` whose valid time does not overlap SHALL stay live. An assert that finds an existing overlapping `(s, p, o)` SHALL return it and retract nothing.

#### Scenario: New value replaces the old one without carrying annotations
- **WHEN** `:age` is `sys:one`, `e1 = (alice :age 30)` has annotation `(e1 :source :form)`, and `(alice :age 31)` is asserted
- **THEN** `e1` and its annotation are retracted with kind `cardinality`
- **AND** the new statement `(alice :age 31)` has no annotations

#### Scenario: Non-overlapping episodes coexist
- **WHEN** `:worksAt` is `sys:one`, `(alice :worksAt acme)` is live valid `[2020-01-01, 2022-01-01)`, and `(alice :worksAt globex)` is asserted valid `[2022-01-01, unbounded)`
- **THEN** both statements are live

#### Scenario: Partial overlap retracts the whole old statement
- **WHEN** `:worksAt` is `sys:one`, `(alice :worksAt acme)` is live valid `[2020-01-01, 2025-01-01)`, and `(alice :worksAt globex)` is asserted valid `[2024-01-01, unbounded)`
- **THEN** the acme statement is retracted as a whole with kind `cardinality`, not trimmed

#### Scenario: Re-asserting the same value
- **WHEN** `:age` is `sys:one`, `(alice :age 30)` is live, and `(alice :age 30)` is asserted again
- **THEN** assert returns `Existing` and nothing is retracted

#### Scenario: Create on a cardinality-one predicate
- **WHEN** `:age` is `sys:one`, `(alice :age 30)` is live, and `(alice :age 30)` is created and then `(alice :age 31)` is created
- **THEN** the second create retracts both `(alice :age 30)` statements with kind `cardinality`, leaving only `(alice :age 31)` live

#### Scenario: Other subjects are untouched
- **WHEN** `:age` is `sys:one` and `(bob :age 40)` is live, and `(alice :age 31)` is asserted
- **THEN** `(bob :age 40)` stays live

### Requirement: Unique predicates allow one live subject per value
For a predicate flagged `sys:unique true`, at most one subject SHALL hold a live statement `(s, p, o)` for any object value `o`, regardless of valid time. An assert or create of `(s2, p, o)` while `(s1, p, o)` is live with `s1 ≠ s2` SHALL fail with `UniqueViolation { p, o, existing: s1 }` and the transaction SHALL leave no trace. The same subject MAY hold several live statements with that value. A retracted holder SHALL NOT block a new subject.

#### Scenario: Second subject rejected
- **WHEN** `:email` is unique, `(alice :email "a@x.org")` is live, and `(bob :email "a@x.org")` is asserted
- **THEN** the transaction fails with `UniqueViolation` naming `:email`, `"a@x.org"` and `alice`
- **AND** no `tx` row, statement or term from that transaction exists

#### Scenario: Valid time does not relax uniqueness
- **WHEN** `:email` is unique, `(alice :email "a@x.org")` is live valid `[2020-01-01, 2021-01-01)`, and `(bob :email "a@x.org")` is asserted valid `[2022-01-01, unbounded)`
- **THEN** the transaction fails with `UniqueViolation`

#### Scenario: Value released by retraction
- **WHEN** `:email` is unique, `(alice :email "a@x.org")` is retracted, and `(bob :email "a@x.org")` is asserted
- **THEN** the statement is inserted

#### Scenario: Same subject new episode
- **WHEN** `:email` is unique and `(alice :email "a@x.org")` is live valid `[2020-01-01, 2021-01-01)`, and the same triple is asserted valid `[2022-01-01, unbounded)`
- **THEN** a second live statement for `alice` is inserted

### Requirement: Upsert on a unique predicate
Upsert of `(p, o)` on a predicate flagged unique SHALL return the subject of the live statement `(s, p, o)` if one exists and write nothing; otherwise it SHALL allocate a new `NODE`, assert `(node, p, o)` without valid time, and return the node. Upsert on a predicate that is not flagged unique SHALL fail with a not-unique-predicate error.

#### Scenario: Existing subject returned
- **WHEN** `:email` is unique and `(alice :email "a@x.org")` is live, and upsert of `(:email, "a@x.org")` is called
- **THEN** it returns `alice` and inserts no statement

#### Scenario: New node created
- **WHEN** `:email` is unique and no live statement holds `"new@x.org"`, and upsert of `(:email, "new@x.org")` is called
- **THEN** it returns a fresh `NODE` id `n`
- **AND** `(n :email "new@x.org")` is live

#### Scenario: Upsert twice in one transaction
- **WHEN** one transaction calls upsert of `(:email, "new@x.org")` twice
- **THEN** both calls return the same node and only one statement is inserted

#### Scenario: Upsert on a non-unique predicate
- **WHEN** upsert is called on a predicate without `sys:unique true`
- **THEN** the transaction fails with a not-unique-predicate error

### Requirement: Value type constrains objects
For a predicate with a live `sys:valueType`, an assert, create or supersede whose object does not match SHALL fail with `ValueTypeMismatch { p, expected, got }`. A tag IRI SHALL match objects with exactly that tag. A datatype IRI SHALL match literals of that datatype in any of their encodings: `xsd:integer` matches `INT` and `TYPED` values of that datatype; `xsd:string` matches `SHORT_STR` and `STR`; `rdf:langString` matches `LANG_STR`; `xsd:boolean`, `xsd:date`, `xsd:dateTime`, `xsd:double` and `xsd:decimal` match `BOOL`, `DATE`, `DATETIME`, `DOUBLE` and `DECIMAL` (and `TYPED` values of the same datatype); any other datatype IRI matches `TYPED` values with that datatype.

#### Scenario: Integer predicate rejects a string
- **WHEN** `:age` has `sys:valueType xsd:integer` and `(alice :age "thirty")` is asserted
- **THEN** the transaction fails with `ValueTypeMismatch` naming `:age`, `xsd:integer` and the string's kind

#### Scenario: Integer predicate accepts large integers
- **WHEN** `:big` has `sys:valueType xsd:integer` and the integer `2^70` is asserted as its object
- **THEN** the statement is inserted

#### Scenario: String datatype accepts short and long strings
- **WHEN** `:name` has `sys:valueType xsd:string` and `"Al"` and `"Alexandrina"` are asserted
- **THEN** both statements are inserted

#### Scenario: Tag constraint on statements
- **WHEN** `:supportedBy` has `sys:valueType sys:STMT` and its object is a `NODE`
- **THEN** the transaction fails with `ValueTypeMismatch`

### Requirement: Edge flag is stored
`sys:isEdge` SHALL be stored and versioned like the other flags and SHALL NOT constrain writes; it only records whether query front ends present the predicate as a relationship or a property.

#### Scenario: isEdge on a literal-valued predicate
- **WHEN** `(:nickname sys:isEdge true)` is live and `(alice :nickname "Al")` is asserted
- **THEN** the statement is inserted and the flag is visible in the now view

### Requirement: Schema changes violated by live data are rejected
Asserting a schema flag that live data already violates SHALL fail with `SchemaConflict { violating }` listing, in ascending order, the eids of every live statement involved in a violation, and the transaction SHALL leave no trace. For `sys:cardinality sys:one`, the violating eids are all live statements of the predicate that share a subject with another live statement of a different object and overlapping valid time. For `sys:unique true`, they are all live statements whose object is held by more than one subject. For `sys:valueType`, they are all live statements whose object does not match. Asserting `sys:cardinality sys:many`, `sys:unique false` or `sys:isEdge`, or retracting any flag, SHALL never conflict. The check SHALL see earlier writes of the same transaction.

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

### Requirement: Order of schema checks
For every inserted statement the checks SHALL run in the order value type, then uniqueness, then cardinality-one replacement, and a failing check SHALL prevent all later ones. For assert, the value-type check SHALL run before the idempotency lookup, and the uniqueness and cardinality steps SHALL run only when a new statement is to be inserted.

#### Scenario: Failed value type prevents cardinality replacement
- **WHEN** `:age` is `sys:one` with `sys:valueType xsd:integer`, `(alice :age 30)` is live, and `(alice :age "thirty")` is asserted
- **THEN** the error is `ValueTypeMismatch`
- **AND** `(alice :age 30)` is still live

#### Scenario: Uniqueness checked before cardinality replacement
- **WHEN** `:email` is unique and `sys:one`, `(alice :email "a@x.org")` and `(bob :email "b@x.org")` are live, and `(bob :email "a@x.org")` is asserted
- **THEN** the error is `UniqueViolation`
- **AND** `(bob :email "b@x.org")` is still live
