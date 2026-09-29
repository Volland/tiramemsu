## Purpose

Defines supersede, the general update verb: it corrects a live statement's object or valid time by retracting the statement's whole cascade set and replaying it under new eids with references rewired, so every annotation and reference survives the correction and history records exactly one correction event.

## ADDED Requirements

### Requirement: Supersede retracts and replays the cascade set
Supersede of a live statement `root` with a patch SHALL, within the current transaction: compute the cascade set `C` of `root` (every live statement reachable from it over subject and object positions, `root` included); allocate a fresh eid for every member of `C`, forming a substitution map `σ` from old to new eids; retract every member of `C` with kind `supersede`; insert, for every member `m`, a new statement with eid `σ(m)`, the same predicate, the subject and object of `m` each replaced by `σ(x)` when they are a member `x` of `C`, the same valid interval, and `t_add` equal to the current transaction, except that the new root takes the patched object and interval; and return the new root eid `σ(root)`.

#### Scenario: Replay rewires annotations and references
- **WHEN** `e1 = (alice worksAt acme)` has annotation `e2 = (e1 :confidence 0.8)` and reference `e7 = (:belief9 :supportedBy e1)`, and `e1` is superseded with `v_from = 2025-02-01`
- **THEN** supersede returns a new eid `e10`
- **AND** the now view contains `e10 = (alice worksAt acme)` valid from `2025-02-01`, `(e10 :confidence 0.8)` and `(:belief9 :supportedBy e10)`, each under a new eid
- **AND** `e1`, `e2` and `e7` are retracted with kind `supersede`

#### Scenario: Deep layers are replayed
- **WHEN** additionally `e8 = (e7 :method "llm-extraction")` is live and `e1` is superseded
- **THEN** the new statement replaying `e8` has as subject the new eid replaying `e7`, and object `"llm-extraction"`

#### Scenario: Non-root content is unchanged
- **WHEN** an annotation in the cascade set has valid interval `[2024-01-01, 2025-01-01)` and the root is superseded with a new object
- **THEN** the replayed annotation keeps the interval `[2024-01-01, 2025-01-01)` and its original predicate and literal object

#### Scenario: Cycles are replayed with both references rewritten
- **WHEN** the cascade set contains two statements that reference each other
- **THEN** their replays reference each other's new eids, and neither references an old eid

### Requirement: Supersede links the new root to the old one
Supersede SHALL insert the statement `(σ(root) sys:supersedes root)` in the same transaction. Because that link has the new root as its subject, it belongs to the new root's cascade set, so a later supersede of the new root SHALL replay earlier links onto the newest root in addition to adding its own link.

#### Scenario: Link after one correction
- **WHEN** `e1` is superseded and the new root is `e10`
- **THEN** the now view contains `(e10 sys:supersedes e1)`

#### Scenario: Chain of corrections
- **WHEN** `e1` is superseded into `e10`, and later `e10` is superseded into `e20`
- **THEN** the now view contains `(e20 sys:supersedes e10)` and `(e20 sys:supersedes e1)`
- **AND** `(e10 sys:supersedes e1)` is retracted with kind `supersede`

### Requirement: Patch rules
A patch SHALL only be able to set the object, set or clear `v_from`, and set or clear `v_to`; each field left out keeps the old root's value. A patch that names the subject or the predicate SHALL be rejected with `InvalidPatch`. A patch whose resulting interval is empty (`v_from ≥ v_to` with both bounds present) SHALL be rejected with `InvalidPatch`. A patch that leaves the object and both bounds equal to the old root's values SHALL be rejected with `InvalidPatch`. Rejection SHALL leave no trace.

#### Scenario: Close an open interval
- **WHEN** `e1 = (alice worksAt acme)` valid `[2020-01-01, unbounded)` is superseded with `v_to = 2026-03-01`
- **THEN** the new root is valid `[2020-01-01, 2026-03-01)`

#### Scenario: Clear a bound
- **WHEN** a statement valid `[2020-01-01, 2021-01-01)` is superseded with `v_to` cleared
- **THEN** the new root is valid `[2020-01-01, unbounded)`

#### Scenario: Change the object
- **WHEN** `(alice :name "Alcie")` is superseded with object `"Alice"`
- **THEN** the new root is `(alice :name "Alice")` with the old valid interval

#### Scenario: Patch makes the interval empty
- **WHEN** a statement valid `[2025-06-01, unbounded)` is superseded with `v_to = 2025-01-01`
- **THEN** the transaction fails with `InvalidPatch` and leaves no trace

#### Scenario: Patch names the subject
- **WHEN** a patch received through a binding sets the subject
- **THEN** the supersede fails with `InvalidPatch`

#### Scenario: Patch changes nothing
- **WHEN** a statement is superseded with an empty patch, or with its current object
- **THEN** the transaction fails with `InvalidPatch`

### Requirement: Only live statements can be superseded
Supersede of a retracted statement, or of an eid that does not exist, SHALL fail with `NotLive(eid)` and leave no trace. This includes a statement retracted earlier in the same transaction, for example by an earlier supersede or cascade.

#### Scenario: Supersede a retracted statement
- **WHEN** `e1` was retracted and is superseded
- **THEN** the transaction fails with `NotLive(e1)`

#### Scenario: Supersede the same statement twice in one transaction
- **WHEN** one transaction supersedes `e1` and then supersedes `e1` again
- **THEN** the second supersede fails with `NotLive(e1)` and the whole transaction leaves no trace

#### Scenario: Supersede a member of an earlier cascade set
- **WHEN** one transaction supersedes `e1` and then supersedes its old annotation `e2`
- **THEN** the second supersede fails with `NotLive(e2)`

### Requirement: Supersede is bounded and schema-checked
The cascade set of a supersede SHALL be bounded by `max_cascade`; a larger set SHALL fail with `CascadeLimitExceeded { root, limit }`. The new root SHALL pass the predicate-schema checks for value type and uniqueness, and SHALL trigger cardinality-one replacement of other live objects of the same subject and predicate whose valid time overlaps the new root. The new root SHALL always be inserted, even if a live statement with equal content exists.

#### Scenario: Cascade set too large
- **WHEN** a statement with 10 annotations is superseded with `max_cascade = 5`
- **THEN** the transaction fails with `CascadeLimitExceeded` and leaves no trace

#### Scenario: Patched object violates a value type
- **WHEN** `:age` has `sys:valueType` `INT` and `(alice :age 30)` is superseded with object `"thirty"`
- **THEN** the transaction fails with `ValueTypeMismatch`

#### Scenario: Patched object collides with a unique value
- **WHEN** `:email` is unique, `(alice :email "a@x.org")` and `(bob :email "b@x.org")` are live, and Bob's statement is superseded with object `"a@x.org"`
- **THEN** the transaction fails with `UniqueViolation` naming `alice`

### Requirement: Supersede report and event log
The report of a transaction with a supersede SHALL list every member of the cascade set in `retracted` with kind `supersede`, every replayed statement and the `sys:supersedes` link in `asserted`, and every `(old, new)` pair of `σ` in `superseded` with the root pair first. In the event log, a supersede SHALL appear as retract events of kind `supersede` and assert events, all at the same transaction.

#### Scenario: Report of a supersede
- **WHEN** `e1` with annotation `e2` is superseded into `e10`, replaying `e2` as `e11`, with link `e12`
- **THEN** `superseded = [(e1, e10), (e2, e11)]`, `retracted = [(e1, supersede), (e2, supersede)]` and `asserted` contains `e10`, `e11` and `e12`

#### Scenario: History shows one correction
- **WHEN** events since the transaction before the supersede are listed
- **THEN** they are exactly the retract events for `e1` and `e2` with kind `supersede` and the assert events for `e10`, `e11` and `e12`, all at the supersede's transaction
