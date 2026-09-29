## Purpose

Defines the write operations that create, retract and corroborate individual statements (assert, create, retract, retract-by-pattern, confirm, new node), the half-open valid-time interval every statement carries, and the rules on value positions, self-reference and the reserved `sys:` namespace that every write must satisfy.

## ADDED Requirements

### Requirement: Statement content and positions
Every statement SHALL consist of an eid (a `STMT` ObjectId), a subject, a predicate, an object and an optional valid-time interval. The subject SHALL be an `IRI`, `NODE`, `BNODE`, `STMT` or `TX` ObjectId; the predicate SHALL be an `IRI`; the object MAY be any non-reserved ObjectId. A write whose value violates these position rules SHALL fail with an invalid-term error. After insertion, a statement's eid, subject, predicate, object and valid interval SHALL never change.

#### Scenario: Literal subject rejected
- **WHEN** a transaction asserts a statement whose subject is the integer 5
- **THEN** the transaction fails with an invalid-term error and leaves no trace

#### Scenario: Non-IRI predicate rejected
- **WHEN** a transaction asserts a statement whose predicate is a `NODE`
- **THEN** the transaction fails with an invalid-term error

#### Scenario: Statement and transaction subjects accepted
- **WHEN** a transaction asserts `(e1 :confidence 0.8)` for a live statement `e1` and `(tx3 :note "ok")` for committed transaction 3
- **THEN** both statements are inserted

### Requirement: Statement references are not checked for liveness
A `STMT` ObjectId used as the subject or object of a written statement SHALL be accepted whether it refers to a live statement, a retracted statement, or an id not yet allocated (a forward reference); the only forbidden reference is the statement's own eid. A statement that references an already-retracted statement SHALL stay live until it is itself retracted, because cascades only run at the moment of a retraction.

#### Scenario: Annotating a retracted statement
- **WHEN** statement `e1` was retracted in an earlier transaction and a transaction asserts `(e1 :note "was wrong")`
- **THEN** the statement is inserted and is live in the now view

#### Scenario: Forward reference closes a cycle
- **WHEN** a transaction creates `e7 = (:b1 :about X)`, where `X` is the `STMT` id the next statement will receive, and then creates `e8 = (e7 :about :b2)`, which receives id `X`
- **THEN** both statements are inserted, and `e7` and `e8` reference each other

### Requirement: Assert is idempotent over overlapping valid time
Assert SHALL return `Existing(eid)` without inserting anything when a live statement with the same subject, predicate and object exists whose valid interval overlaps the requested one; otherwise it SHALL insert a new statement with a fresh eid, `t_add` equal to the current transaction and the requested valid interval, and return `New(eid)`. Intervals `a` and `b` overlap exactly when `(a.from unbounded OR b.to unbounded OR a.from < b.to) AND (b.from unbounded OR a.to unbounded OR b.from < a.to)`. When several live statements match, the one with the smallest eid SHALL be returned. An overlapping but different interval SHALL NOT be merged, widened or narrowed.

#### Scenario: Same fact twice
- **WHEN** `(alice worksAt acme)` without valid time is asserted in transaction 1 and again in transaction 2
- **THEN** transaction 2 returns `Existing` with the eid from transaction 1
- **AND** the `triple` table gains no row in transaction 2

#### Scenario: Overlapping bounded intervals
- **WHEN** `(alice worksAt acme)` is live with `[2020-01-01, 2022-01-01)` and the same triple is asserted with `[2021-01-01, 2023-01-01)`
- **THEN** assert returns `Existing` with the first eid
- **AND** the stored interval stays `[2020-01-01, 2022-01-01)`

#### Scenario: Touching intervals do not overlap
- **WHEN** `(alice worksAt acme)` is live with `[2020-01-01, 2022-01-01)` and the same triple is asserted with `[2022-01-01, 2024-01-01)`
- **THEN** assert returns `New` with a second eid and both statements are live

#### Scenario: Unbounded interval overlaps everything
- **WHEN** `(alice worksAt acme)` is live with no valid time and the same triple is asserted with `[1990-01-01, 1991-01-01)`
- **THEN** assert returns `Existing` with the first eid

#### Scenario: Half-open ends
- **WHEN** `(alice worksAt acme)` is live with `[2024-01-01, unbounded)` and the same triple is asserted with `[unbounded, 2024-01-01)`
- **THEN** the intervals do not overlap and assert returns `New`

#### Scenario: Non-overlapping episodes coexist
- **WHEN** `(alice worksAt acme)` is asserted with `[2020-01-01, 2022-06-01)` and then with `[2024-01-01, unbounded)`
- **THEN** two distinct live eids exist for the same triple

#### Scenario: Retracted match is ignored
- **WHEN** `(alice worksAt acme)` was retracted and is asserted again
- **THEN** assert returns `New` with a fresh eid different from the retracted one

### Requirement: Assert can confirm an existing match
Assert SHALL accept an on-existing policy with values `Return` (default) and `Confirm`. With `Confirm`, an assert that finds an existing match SHALL additionally record a confirmation of the matched eid by the current transaction, exactly as the confirm operation does, and still return `Existing(eid)`. With `Confirm`, an assert that inserts a new statement SHALL NOT record a confirmation.

#### Scenario: Confirming re-assert
- **WHEN** `(alice worksAt acme)` is live as `e1` and transaction 9 asserts it again with policy `Confirm`
- **THEN** assert returns `Existing(e1)`
- **AND** the now view contains `(e1 sys:confirmedBy tx9)`

#### Scenario: Confirm policy on a new statement
- **WHEN** a triple that is not live is asserted with policy `Confirm`
- **THEN** assert returns `New(eid)` and no `sys:confirmedBy` statement is written for it

### Requirement: Create always inserts
Create SHALL always insert a new statement with a fresh eid, even when a live statement with the same subject, predicate, object and an overlapping valid interval exists, and SHALL return the new eid. Predicate-schema checks SHALL apply to create exactly as to assert.

#### Scenario: Parallel edges
- **WHEN** `(a called b)` is created twice
- **THEN** two different eids are returned and both statements are live

#### Scenario: Create after assert
- **WHEN** `(a called b)` is asserted as `e1` and then created
- **THEN** create returns a new eid `e2 ≠ e1` and both are live

### Requirement: Valid time is a half-open interval
A statement's valid interval SHALL be `[v_from, v_to)` in epoch milliseconds, where an absent bound is unbounded and a statement with both bounds absent is valid for all time. When both bounds are present, `v_from` SHALL be strictly less than `v_to`; an assert or create with `v_from ≥ v_to` SHALL fail with an invalid-interval error. A statement valid `[a, b)` SHALL be valid at `a` and at every instant before `b`, and SHALL NOT be valid at `b`.

#### Scenario: Empty interval rejected
- **WHEN** a statement is asserted with `v_from = v_to = 2025-01-01`
- **THEN** the transaction fails with an invalid-interval error and leaves no trace

#### Scenario: Reversed interval rejected
- **WHEN** a statement is created with `v_from = 2026-01-01` and `v_to = 2025-01-01`
- **THEN** the transaction fails with an invalid-interval error

#### Scenario: One-sided intervals accepted
- **WHEN** statements are asserted with only `v_from`, and with only `v_to`
- **THEN** both are inserted with the absent bound stored as unbounded

#### Scenario: End instant is excluded
- **WHEN** a statement is valid `[2025-01-01, 2026-03-01)`
- **THEN** it is valid at `2025-01-01T00:00:00.000Z` and at `2026-02-28T23:59:59.999Z`
- **AND** it is not valid at `2026-03-01T00:00:00.000Z`

### Requirement: Retract sets the retraction exactly once
Retract SHALL, for a live statement, set `t_ret` to the current transaction and `ret_kind` to explicit, cascade the retraction, and return true. For a statement that is already retracted, or an eid that does not exist, retract SHALL change nothing and return false. A retraction SHALL never be undone or overwritten.

#### Scenario: First retraction
- **WHEN** live statement `e1` is retracted in transaction 4
- **THEN** retract returns true and `e1` has `t_ret = 4` and `ret_kind = explicit`

#### Scenario: Second retraction is a no-op
- **WHEN** `e1`, retracted in transaction 4, is retracted again in transaction 6
- **THEN** retract returns false
- **AND** `e1` keeps `t_ret = 4` and `ret_kind = explicit`, and the report of transaction 6 does not list `e1`

#### Scenario: Retract twice in one transaction
- **WHEN** one transaction retracts `e1` twice
- **THEN** the first call returns true and the second returns false

#### Scenario: Unknown eid
- **WHEN** a statement id that was never allocated is retracted
- **THEN** retract returns false and the transaction can still commit

#### Scenario: Assert and retract in the same transaction
- **WHEN** one transaction, number 7, asserts `e5` and then retracts it
- **THEN** `e5` has `t_add = 7` and `t_ret = 7`
- **AND** `e5` is absent from the now view and from every as-of view, and present in the history view

### Requirement: Retract by pattern
Retract-matching SHALL take an optional subject, predicate and object, find every statement that is live at the moment of the call and matches all given positions regardless of valid time, retract each of them that is still live in ascending eid order (with cascade), and return the matched eids. A matched statement already retracted by the cascade of an earlier match in the same call SHALL keep the retraction kind of that cascade. A pattern with no match SHALL return an empty list and change nothing.

#### Scenario: Retract all objects of a subject and predicate
- **WHEN** `(alice likes tea)` and `(alice likes coffee)` are live and retract-matching is called with subject `alice` and predicate `likes`
- **THEN** both eids are returned and both are retracted with kind explicit
- **AND** `(alice worksAt acme)` stays live

#### Scenario: Valid time does not filter
- **WHEN** `(alice worksAt acme)` has two live episodes with disjoint valid intervals and retract-matching is called with that full triple
- **THEN** both episodes are retracted

#### Scenario: Match reached through an earlier cascade
- **WHEN** live `e2 = (e1 :note "x")` annotates live `e1 = (alice :note "y")`, and retract-matching is called with predicate `:note`
- **THEN** both `e1` and `e2` are returned
- **AND** `e1` has kind explicit and `e2` has kind cascade

#### Scenario: Nothing matches
- **WHEN** retract-matching is called with a subject that has no live statements
- **THEN** it returns an empty list and the transaction still commits

### Requirement: Confirm records corroboration
Confirm SHALL, for a live statement `eid`, assert the statement `(eid sys:confirmedBy txT)` where `txT` is the current transaction, change nothing else, and return the eid of that confirmation statement. Confirming the same eid twice in one transaction SHALL return the same confirmation eid. Confirming a retracted or non-existent eid SHALL fail with `NotLive(eid)`.

#### Scenario: Confirmation by two transactions
- **WHEN** live `e1` is confirmed in transaction 5 and again in transaction 8
- **THEN** two distinct statements `(e1 sys:confirmedBy tx5)` and `(e1 sys:confirmedBy tx8)` are live
- **AND** `e1` itself is unchanged

#### Scenario: Confirm a retracted statement
- **WHEN** `e1` is retracted and a later transaction confirms it
- **THEN** the transaction fails with `NotLive(e1)` and leaves no trace

### Requirement: New nodes
The new-node operation SHALL allocate and return a fresh `NODE` ObjectId that has never been issued before in the database, without writing any statement. A node SHALL have no lifetime of its own: it exists in a view only through statements that mention it, and it SHALL never be retracted or cascaded.

#### Scenario: Fresh node ids
- **WHEN** a transaction calls new-node twice
- **THEN** two different `NODE` ObjectIds are returned
- **AND** no statement is written for them

#### Scenario: Retracting statements about a node does not touch other statements about it
- **WHEN** node `n` has statements `(n :name "x")` and `(n :age 3)` and the first is retracted
- **THEN** `(n :age 3)` stays live

### Requirement: No direct self-reference
A statement SHALL NOT use its own eid as its subject or object. A write that would do so SHALL fail with `SelfReference(eid)`. Longer reference cycles between different statements SHALL be allowed.

#### Scenario: Guessed own eid
- **WHEN** a caller asserts a statement whose object is the `STMT` ObjectId that the engine will allocate for that very statement
- **THEN** the transaction fails with `SelfReference` carrying that eid and leaves no trace

#### Scenario: Longer cycles allowed
- **WHEN** two statements are written so that each uses the other's eid as its subject or object
- **THEN** both are inserted, because neither references itself

### Requirement: Reserved sys namespace
User writes SHALL NOT use a predicate in the `urn:tiramemsu:sys:` namespace, except: the schema flags `sys:cardinality`, `sys:unique`, `sys:valueType` and `sys:isEdge`; the vocabulary settings `sys:vocab`, `sys:prefix`, `sys:prefixName` and `sys:prefixIri`; and the transaction-metadata predicates `sys:author`, `sys:source` and `sys:reason` when the subject is a transaction. Any other `sys:` predicate, including `sys:confirmedBy`, `sys:supersedes`, `sys:subject`, `sys:object` and `sys:predicate`, SHALL be rejected with `ReservedNamespace(iri)`. The engine itself SHALL write `sys:confirmedBy` and `sys:supersedes`. `sys:` IRIs in subject or object position and retraction of `sys:` statements SHALL be allowed.

#### Scenario: Forging a confirmation
- **WHEN** a transaction asserts `(e1 sys:confirmedBy tx3)` directly
- **THEN** the transaction fails with `ReservedNamespace(urn:tiramemsu:sys:confirmedBy)`

#### Scenario: Unknown sys predicate
- **WHEN** a transaction asserts `(alice sys:foo 1)`
- **THEN** the transaction fails with `ReservedNamespace(urn:tiramemsu:sys:foo)`

#### Scenario: Allowed schema and vocabulary flags
- **WHEN** a transaction asserts `(:email sys:unique true)` and `(sys:db sys:vocab <https://example.org/>)`
- **THEN** both statements are inserted

#### Scenario: Metadata predicate on a non-transaction subject
- **WHEN** a transaction asserts `(alice sys:reason "x")`
- **THEN** the transaction fails with `ReservedNamespace(urn:tiramemsu:sys:reason)`

#### Scenario: Other namespaces are not reserved by writes
- **WHEN** a transaction asserts a statement whose predicate is in `urn:tiramemsu:tm:`
- **THEN** the statement is inserted

### Requirement: Eids are never reused
Every statement inserted by any operation SHALL receive a `STMT` ObjectId that has never been issued before in the database, including ids issued inside speculative transactions and dry runs.

#### Scenario: Retract then re-assert
- **WHEN** `e1 = (alice worksAt acme)` is retracted and the same triple is asserted again
- **THEN** the new statement has an eid different from `e1`, and `e1` remains retracted
