## ADDED Requirements

### Requirement: Derive a belief with support
`Tx::derive(s, p, o, valid, support)` SHALL insert or reuse a statement `D = (s, p, o)` with valid interval `valid` and, for each supporting statement `e`, a link `(D sys:derivedFrom e)` in the same transaction. `support` SHALL be non-empty, every member SHALL be a live statement, and `D` SHALL NOT be its own support; otherwise the transaction SHALL fail with `InvalidTerm` or `NotLive` and leave no trace. `sys:derivedFrom` SHALL be engine-owned: asserting or creating it directly SHALL fail with `ReservedNamespace`. Schema checks SHALL apply to `D` as they do to `create`.

#### Scenario: Summary with two supports
- **WHEN** e1 = `(v:alice v:ordered v:tea)` and e2 = `(v:alice v:ordered v:tea2)` are live and `derive(v:alice, v:prefers, v:tea, ALWAYS, [e1, e2])` runs
- **THEN** a statement D is inserted with links `(D sys:derivedFrom e1)` and `(D sys:derivedFrom e2)`

#### Scenario: Retracted support refused
- **WHEN** e1 is retracted and `derive(…, [e1])` runs
- **THEN** the transaction fails with `NotLive` naming e1

#### Scenario: Direct link refused
- **WHEN** `(D sys:derivedFrom e1)` is asserted with `Tx::assert`
- **THEN** the transaction fails with `ReservedNamespace`

### Requirement: Derivation idempotency
`derive` SHALL return `Existing(D)` when a live statement with the same `(s, p, o)`, an overlapping valid interval and exactly the same set of live supports exists. Otherwise it SHALL create a new statement, also when a live identical statement exists that is not derived or is derived from a different support set, so an independently asserted statement never acquires support links.

#### Scenario: Re-deriving is a no-op
- **WHEN** `derive(v:alice, v:prefers, v:tea, ALWAYS, [e1, e2])` runs twice
- **THEN** the second call returns `Existing` with the first statement's eid and adds no link

#### Scenario: Independent fact stays independent
- **WHEN** `(v:alice v:prefers v:tea)` was asserted directly as e9, then `derive(v:alice, v:prefers, v:tea, ALWAYS, [e1])` runs, and later e1 is retracted
- **THEN** the derive created a new statement D ≠ e9, D is retracted with e1, and e9 stays live

### Requirement: Support cascade
When a support of a live derived statement is retracted by any operation, the derived statement SHALL be retracted in the same transaction, recursively, together with its own cascade set. Support SHALL be conjunctive within one derivation and disjunctive across parallel derivations of the same content.

#### Scenario: Losing one support
- **WHEN** D is derived from e1 and e2, D has a layer `(D v:confidence 0.7)`, and e1 is retracted explicitly
- **THEN** e1, the link `(D sys:derivedFrom e1)`, D, its other link and its confidence layer are retracted in that transaction, all except e1 with kind `cascade`

#### Scenario: Parallel derivations
- **WHEN** D1 = `(v:alice v:prefers v:tea)` is derived from [e1] and D2 with the same content from [e3], and e1 is retracted
- **THEN** D1 is retracted and D2 stays live, so `ASK { v:alice v:prefers v:tea }` is still true

#### Scenario: Stale saved answer
- **WHEN** a saved SPARQL answer cites D, and a support of D is retracted
- **THEN** `check_saved_answers` marks it `Stale` with cause `supportRetracted` naming D's retraction

### Requirement: Supersede and derived statements
When a support is superseded, every derived statement reached through a support link from its cascade set, other than the root, SHALL be retracted with kind `supersede` and SHALL NOT be replayed, and neither SHALL any statement that depends on it structurally. When a derived statement is itself superseded, its support links SHALL be replayed onto the new eid.

#### Scenario: Correcting evidence retracts the summary
- **WHEN** D is derived from e1 and e1 is superseded with a new object
- **THEN** e1 and D are retracted with kind `supersede`, e1's replacement is live, and no replacement of D exists

#### Scenario: Correcting the summary keeps its support
- **WHEN** D is derived from e1 and D is superseded with new valid time
- **THEN** the new statement D′ is live with `(D′ sys:derivedFrom e1)`

### Requirement: Derivations travel in bundles
A fact bundle of a derived statement SHALL contain its support links and their supports, and importing it SHALL write the derived statement through `derive` with its supports mapped to their target eids, with derivation idempotency.

#### Scenario: Bundle round trip
- **WHEN** `View::bundle(D)` of a derived statement is imported into an empty file twice
- **THEN** the target holds D's content derived from the imported supports, and the second import changes nothing

### Requirement: Derive on every surface
The JSON bridge SHALL accept `derive` as a transaction operation, Node and Python SHALL expose it on their transaction builders, and the MCP server SHALL provide a write tool `derive` that is absent in read-only mode.

#### Scenario: MCP read-only mode
- **WHEN** the MCP server runs with `--read-only`
- **THEN** `derive` is not listed and calling it fails with `ReadOnly`
