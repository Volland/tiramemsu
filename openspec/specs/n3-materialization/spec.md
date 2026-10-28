# n3-materialization Specification

## Purpose
TBD - created by archiving change add-n3-reasoning. Update Purpose after archive.

## Requirements

### Requirement: Materialize a rule set
`Db::materialize(rule_set, &MaterializeOptions)` SHALL reason with the live rule set over a read view taken at `t₀` (scope, window and limits as for `reason`, default window unbounded), excluding statements derived from the rule set's live statement `R`, and then write the conclusions in one write transaction through `derive`, in proof order. Each conclusion SHALL be derived with support = the eids of its loaded premises, the eids written for its derived premises, and `R`, and SHALL receive the layers `(D n3:derivedBy rule_set)` and `(D n3:rule i)`. The report SHALL list `reasoned_at` (`t₀`), the commit's `TxReport`, the derived, existing and widened statements, `stale_support` and the status. With `dry_run` nothing SHALL be committed.

#### Scenario: Materialized conclusion
- **WHEN** e1 = `(v:socrates rdf:type v:Human)` is live, the mortality rule set is stored as R, and `materialize(v:mortality)` runs
- **THEN** D = `(v:socrates rdf:type v:Mortal)` is live with links `(D sys:derivedFrom e1)` and `(D sys:derivedFrom R)` and the layer `(D n3:derivedBy v:mortality)`

#### Scenario: Re-running is a no-op
- **WHEN** `materialize(v:mortality)` runs twice without other writes
- **THEN** the second run reports D as existing and its commit asserts nothing

#### Scenario: Dry run
- **WHEN** `materialize` runs with `dry_run`
- **THEN** the report lists what would be derived and the store is unchanged

### Requirement: Conclusions fall with premises and rules
A materialized conclusion SHALL be retracted by the support cascade when any of its premises or its rule-set statement is retracted or superseded, in the same transaction.

#### Scenario: Retracting a premise
- **WHEN** D was materialized from e1 and e1 is retracted
- **THEN** D is retracted in the same transaction with kind `cascade`

#### Scenario: Editing the rules
- **WHEN** D was materialized by rule set version A and `store_rules` stores version B
- **THEN** D is retracted with kind `supersede` in that transaction, and a following `materialize` derives the conclusions of B

### Requirement: Own conclusions are not input
A run SHALL NOT load statements derived from its own rule set's live `R`. Statements derived from other rule sets SHALL be loaded like any statement.

#### Scenario: Layered rule sets
- **WHEN** rule set A concludes `(v:socrates rdf:type v:Mortal)` and rule set B has `{ ?x a v:Mortal } => { ?x v:has v:lifespan }`, and A then B are materialized
- **THEN** B's conclusion has A's conclusion among its supports, and re-materializing A changes nothing

### Requirement: Only monotonic rule sets materialize
`materialize` SHALL fail with `N3NotMaterializable { rule, builtin }` for a rule set classified `NonMonotonic`, `ClockDependent` or `UntracedPremises`, before reasoning, and SHALL fail with `N3Contradiction { rule }` before writing when an inference fuse fires.

#### Scenario: Negation refused
- **WHEN** `materialize` runs a rule set that uses `log:notIncludes`
- **THEN** it fails with `N3NotMaterializable` naming the rule and the builtin, and nothing is written

#### Scenario: Contradiction aborts
- **WHEN** a fuse rule fires during `materialize`
- **THEN** it fails with `N3Contradiction` and nothing is written

### Requirement: Valid time of materialized conclusions
Each merged conclusion of the sweep SHALL be derived with its interval as valid time. Conclusions with the same content and different support SHALL be separate statements. When a live statement derived from exactly the same support has the same content and an interval that overlaps or touches the new one without containing it, it SHALL be superseded to the hull of both intervals and reported as widened; when it contains the new interval it SHALL be reported as existing.

#### Scenario: Two episodes from different support
- **WHEN** `(v:alice v:team v:a)` is valid `[2020, 2022)`, `(v:alice v:team v:b)` is valid `[2022, 2024)`, both teams are `v:locatedIn v:berlin`, and a rule concludes `?p v:worksIn ?l` from team and location
- **THEN** two statements `(v:alice v:worksIn v:berlin)` exist, valid `[2020, 2022)` and `[2022, 2024)`, each with its own support

#### Scenario: Widening across windows
- **WHEN** `materialize` runs with window `[2020, 2022)`, then without a window, over facts valid `[2020, 2024)`
- **THEN** the conclusion is first valid `[2020, 2022)`, then superseded to `[2020, 2024)` with the same support, and reported as widened

#### Scenario: Require complete
- **WHEN** a run hits `max_segments` with `require_complete: true`
- **THEN** `materialize` fails with `Incomplete` and nothing is written

### Requirement: Snapshot then commit
Reasoning SHALL run outside the writer; only the commit holds it. A conclusion whose support is no longer live at commit time SHALL be skipped together with every conclusion that depends on it, and reported in `stale_support`; it SHALL NOT fail the run.

#### Scenario: Premise retracted during reasoning
- **WHEN** e1 is retracted by another writer after `t₀` and before the commit
- **THEN** the conclusion resting on e1 is reported in `stale_support` and not written, and other conclusions are written

### Requirement: Skolem nodes
A blank node in a conclusion SHALL become the node returned by `upsert_derived(n3:skolem, "<R eid>/<rule>/<label>", support)`, where `label` is eyeron's deterministic witness name and `support` the support of the rule firing, so re-running reuses the node.

#### Scenario: Stable invented node
- **WHEN** `{ ?x a v:Employee } => { ?x v:manager _:m }` is materialized twice for `v:bob`
- **THEN** both runs use the same node for `_:m` and the second asserts nothing

### Requirement: Materialization surfaces
The JSON bridge SHALL accept a database operation `materialize`, Node, Python and wasm SHALL expose it, the MCP server SHALL provide a write tool `materialize` absent in read-only mode, and the CLI SHALL provide `tm materialize <file> <iri> [--valid-during <from>..<to>] [--graph <iri>]… [--require-complete] [--dry-run]`.

#### Scenario: CLI dry run
- **WHEN** `tm materialize memory.db urn:tiramemsu:v:mortality --dry-run` runs
- **THEN** it prints what would be derived and the file is unchanged
