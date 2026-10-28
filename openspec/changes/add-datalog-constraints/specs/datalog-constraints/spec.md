## ADDED Requirements

### Requirement: Enforce a program
`TxDatalog::enforce(program, true)` SHALL assert `(program dl:enforced true)` for a stored program, and `enforce(program, false)` SHALL retract it. Turning enforcement on SHALL first check every `error` constraint of the program against the transaction's state and fail with `ConstraintViolated` listing every violation if any exists. Enforcement SHALL follow transaction time: superseding an enforced program's text SHALL re-check it in the same transaction, and enforcement as of a past transaction SHALL be what was believed then.

#### Scenario: Enforce clean data
- **WHEN** a program with `constraint one_ceo error :- ex:role(X, ex:CEO), ex:role(Y, ex:CEO), X != Y.` is stored and enforced over a store with one CEO
- **THEN** the transaction commits and `(program dl:enforced true)` is live

#### Scenario: Enforce over violating data
- **WHEN** the store already has two overlapping CEOs and the program is enforced
- **THEN** the transaction fails with `ConstraintViolated` listing both bindings, and nothing is written

#### Scenario: Supersede re-checks
- **WHEN** an enforced program's text is superseded by one with a constraint the live data violates
- **THEN** the transaction fails with `ConstraintViolated`

### Requirement: Error constraints are checked at commit
Before a transaction commits, every `error` constraint of every enforced program SHALL be evaluated over the transaction's final state for the violations that the transaction introduces; if any exists the transaction SHALL roll back with `ConstraintViolated { program, constraint, violations }`, each violation with its bindings, eids and valid interval set. A violation that existed before the transaction and is unchanged by it SHALL NOT fail it. `warn` constraints SHALL NOT fail a transaction; the report SHALL list the `warn` violations it introduces.

#### Scenario: Second CEO refused
- **WHEN** `one_ceo` is enforced, `(ex:a ex:role ex:CEO)` is valid always, and a transaction asserts `(ex:b ex:role ex:CEO)` valid `[2025, ∞)`
- **THEN** it fails with `ConstraintViolated` for `one_ceo` with interval `[2025, ∞)`, and `ex:b` is not CEO

#### Scenario: Successor accepted
- **WHEN** the same transaction also closes `ex:a`'s statement at 2025 by supersede
- **THEN** it commits

#### Scenario: Negation reacts to retraction
- **WHEN** `constraint owned error :- rdf:type(P, ex:Project), not ex:owner(P, _).` is enforced and a transaction retracts a project's only owner
- **THEN** it fails with `ConstraintViolated` for `owned`

#### Scenario: Warn reports without failing
- **WHEN** `owned` is declared `warn` instead and the same retraction runs
- **THEN** the transaction commits and its report lists the `owned` violation

#### Scenario: Metric window
- **WHEN** `constraint cooldown error :- ex:dose(P, _), past[1ms, 6h] ex:dose(P, _).` is enforced, doses are one-millisecond statements, and a dose is asserted 2 hours after another
- **THEN** the transaction fails with `ConstraintViolated` for `cooldown`

### Requirement: Checks read only the change
A commit-time check SHALL evaluate each constraint only from bindings seeded by the transaction's added statements matching a positive atom or retracted statements matching a negated atom (within the reach of the constraint's metric windows), and SHALL report in the transaction report the number of constraints checked and of seed rows read. A transaction that adds or retracts no statement of a predicate a constraint reads SHALL NOT evaluate it.

#### Scenario: Unrelated write skips the check
- **WHEN** `one_ceo` is enforced and a transaction asserts only `ex:name` statements
- **THEN** the report shows zero constraints checked

#### Scenario: Seed rows follow the write
- **WHEN** a store of 10⁵ `ex:role` statements receives one new `ex:role` statement
- **THEN** the report shows one seed row for `one_ceo`

### Requirement: Enforceable constraints
An `error` constraint SHALL be enforceable when its body reads only stored atoms, reserved atoms, derived relations that are not recursive (inlined into the body) and metric operators with finite windows. Enabling enforcement of a program with a non-enforceable `error` constraint SHALL fail with `ConstraintNotEnforceable` naming it; `View::check` SHALL still evaluate it.

#### Scenario: Recursive constraint not enforceable
- **WHEN** a program's constraint `no_cycle error :- reports(X, X).` reads the recursive relation `reports` and the program is enforced
- **THEN** enforcement fails with `ConstraintNotEnforceable` naming `no_cycle`

#### Scenario: Non-recursive relation inlined
- **WHEN** `ceo(X) :- ex:role(X, ex:CEO).` and `constraint one_ceo error :- ceo(X), ceo(Y), X != Y.` are enforced
- **THEN** enforcement succeeds and the second-CEO write is refused as before

### Requirement: Constraint check budget
Commit-time checks SHALL run under `OpenOptions::constraint_budget` (a deadline and a seed-row limit, by default 50 ms and 100 000 rows, both configurable per transaction through `TxOptions`). Exceeding it SHALL roll the transaction back with `ConstraintBudgetExceeded { program, constraint }`.

#### Scenario: Budget fails closed
- **WHEN** a transaction's checks exceed a seed-row budget of 10
- **THEN** it fails with `ConstraintBudgetExceeded` and nothing is written

### Requirement: Previews report violations
A dry-run transaction SHALL run the checks and return, instead of failing, a report listing the `error` violations it would cause. A `with` speculation SHALL fail with `ConstraintViolated` as a real commit would.

#### Scenario: Dry run lists the violation
- **WHEN** the second-CEO assert runs with `dry_run: true`
- **THEN** the report lists the `one_ceo` violation and nothing is written

### Requirement: Same verdict on every backend
For the same history and transaction, the check SHALL accept or refuse identically on SQLite and redb, with identical violations.

#### Scenario: Random histories agree
- **WHEN** random histories with an enforced program are replayed on SQLite and redb
- **THEN** the same transactions are refused with the same violations

### Requirement: Constraint surfaces
The JSON bridge SHALL offer the tx op `enforce` (`program`, `on`) and SHALL report `constraintChecks` and `constraintViolations` in transaction reports and the error code `ConstraintViolated` with its violations; Node, Python and wasm SHALL expose them. The MCP server SHALL offer the write tool `enforce_program`, hidden in read-only mode. The CLI SHALL offer `tm program enforce <file> <program>` and `tm program unenforce <file> <program>`.

#### Scenario: Bridge refusal carries violations
- **WHEN** a bridge transaction violates an enforced constraint
- **THEN** the error has code `ConstraintViolated` and a `violations` array with bindings, eids and intervals

#### Scenario: CLI enforce
- **WHEN** `tm program enforce mem.db ex:policies` runs on clean data
- **THEN** it exits with status 0 and `tm program show` reports the program as enforced
