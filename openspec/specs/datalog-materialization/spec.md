# datalog-materialization Specification

## Purpose
Materializing Datalog programs as derived beliefs: what is written and with which support, how valid time, widening and invented nodes work, and how non-monotonic strata are reconciled.

## Requirements

### Requirement: Materialize a program
`Db::materialize_program(program, &MaterializeProgramOptions)` SHALL evaluate the live stored program over a read view taken at `t₀` (scope, window and limits as for `datalog_reason`) and write, in one write transaction through `derive` and in proof order, every conclusion whose predicate is a stored-predicate IRI. Each conclusion SHALL be derived with support = the eids of its stored premises, the eids written for its derived premises and the eid of the program's live `dl:source` statement `R`, and SHALL receive the layers `(D dl:derivedBy program)` and `(D dl:rule i)`. The transaction SHALL carry `(tx dl:materialized program)`. The report SHALL list `reasoned_at`, the commit's `TxReport`, the derived, existing, widened, reconciled, `stale_support` and `unstorable` conclusions, the predicates of non-monotonic strata as `needs_rerun_after_writes`, and the status. With `dry_run` nothing SHALL be committed.

#### Scenario: Materialized chain
- **WHEN** e1 = `(alice manager bob)` and e2 = `(bob manager carol)` are live and a program with `ex:reportsTo(X, Y) :- ex:manager(X, Y).` and `ex:reportsTo(X, Z) :- ex:reportsTo(X, Y), ex:manager(Y, Z).` is materialized
- **THEN** three `ex:reportsTo` statements are live; `(alice reportsTo carol)` has the support links to the statement written for `(alice reportsTo bob)`, to e2 and to R

#### Scenario: Re-running is a no-op
- **WHEN** the program is materialized twice without other writes
- **THEN** the second run reports every conclusion as existing and asserts nothing but its `dl:materialized` label

#### Scenario: Dry run
- **WHEN** it runs with `dry_run`
- **THEN** the report lists what would be derived and the store is unchanged

#### Scenario: Unstorable relation
- **WHEN** the program also defines a plain-named relation `reach(X, Y)`
- **THEN** `reach` is not written and is not reported as unstorable; a conclusion with a literal subject is reported as unstorable

### Requirement: Conclusions fall with premises and programs
A materialized conclusion SHALL be retracted by the support cascade when any of its premises or the program statement `R` is retracted or superseded, in the same transaction.

#### Scenario: Retracting a premise
- **WHEN** e2 is retracted
- **THEN** `(bob reportsTo carol)` and `(alice reportsTo carol)` are retracted in the same transaction

#### Scenario: Editing the program
- **WHEN** the program's text is superseded
- **THEN** every conclusion it materialized is retracted with kind `supersede` in that transaction

### Requirement: Own conclusions are not input
A full run SHALL NOT read statements derived from its own live `R`. Statements derived from other programs or N3 rule sets SHALL be read like any statement.

#### Scenario: Layered programs
- **WHEN** program A concludes `ex:reportsTo` and program B reads `ex:reportsTo`, and A then B are materialized
- **THEN** B's conclusions have A's conclusions among their supports, and re-materializing A changes nothing

### Requirement: Reconcile non-monotonic strata
Strata that use negation or aggregates SHALL be materialized. A run SHALL retract, by ordinary retraction in its transaction, each live conclusion of the program from such a stratum that the run does not derive with the same content, support and interval, SHALL label the transaction `(tx dl:reconciled program)` with the count, and SHALL report those strata in `needs_rerun_after_writes`.

#### Scenario: Unmanaged becomes managed
- **WHEN** `ex:unmanaged(X) :- rdf:type(X, ex:Employee), not ex:manager(X, _).` is materialized for `bob`, then `(bob manager carol)` is asserted, then the program is materialized again
- **THEN** the second run retracts `(bob unmanaged true)`, the transaction carries `dl:reconciled` with count 1, and the report lists the stratum in `needs_rerun_after_writes`

#### Scenario: Aggregates track headcount
- **WHEN** a headcount program is materialized, an employee is added, and it is materialized again
- **THEN** the old `ex:headcount` statement is retracted and the new count is derived

#### Scenario: Monotonic strata are not reconciled
- **WHEN** a monotonic conclusion's only support is retracted and the program is materialized again
- **THEN** the cascade has already retracted it and the run's report shows no reconciliation for that stratum

### Requirement: Valid time of materialized conclusions
Each maximal interval fragment of a conclusion that rests on one derivation SHALL be derived as one statement with that interval as valid time; fragments with different support SHALL be separate statements. When a live statement derived from exactly the same support has the same content and an interval that overlaps or touches the new one without containing it, it SHALL be superseded to the hull and reported as widened; when it contains the new interval it SHALL be reported as existing. With `require_complete`, a run that reached a limit SHALL fail with `DatalogIncomplete` and write nothing.

#### Scenario: Temporal conclusion
- **WHEN** `ex:alert(X, true) :- past[0d, 3d] ex:fever(X, true).` is materialized over a fever valid `[day 10, day 11)`
- **THEN** `(p alert true)` is live, valid `[day 10, day 14)`, supported by the fever statement and R

#### Scenario: Widening across windows
- **WHEN** a program is materialized with window `[2020, 2022)`, then without a window, over facts valid `[2020, 2024)`
- **THEN** the conclusion is first valid `[2020, 2022)` and then superseded to `[2020, 2024)` with the same support, reported as widened

#### Scenario: Require complete
- **WHEN** a run reaches `max_facts` with `require_complete`
- **THEN** `materialize_program` fails with `DatalogIncomplete` and nothing is written

### Requirement: Snapshot then commit
Evaluation SHALL run outside the writer; only the commit holds it. A conclusion whose support is no longer live at commit time SHALL be skipped together with every conclusion that depends on it and reported in `stale_support`, without failing the run.

#### Scenario: Premise retracted during evaluation
- **WHEN** e1 is retracted by another writer after `t₀` and before the commit
- **THEN** conclusions resting on e1 are reported in `stale_support` and not written, and the others are written

### Requirement: Invented nodes
A node invented by `exists` SHALL become the node returned by `upsert_derived(dl:skolem, "<R eid>/<rule>/<frontier values>", support)`, so re-running the same firing reuses the node.

#### Scenario: Stable invented manager
- **WHEN** `exists M: ex:assignedManager(X, M) :- rdf:type(X, ex:Employee), not ex:manager(X, _).` is materialized twice for `bob`
- **THEN** both runs use the same node and the second asserts no new `ex:assignedManager` statement

### Requirement: Materialization surfaces
The JSON bridge SHALL accept a database op `materializeProgram` (`program`, `scope`, `window`, `limits`, `requireComplete`, `dryRun`, `budget`); Node, Python and wasm SHALL expose it; the MCP server SHALL offer a write tool `materialize_program`, absent in read-only mode; the CLI SHALL offer `tm materialize-program <file> <program> [--dry-run] [--require-complete] [--window <from>..<to>] [--graph <iri>]…`.

#### Scenario: CLI dry run
- **WHEN** `tm materialize-program mem.db ex:reporting --dry-run` runs
- **THEN** it prints what would be derived and reconciled and the file is unchanged
