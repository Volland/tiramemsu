## ADDED Requirements

### Requirement: Enforce an ELPI program
`TxElpi::enforce_elpi(program, true)` SHALL assert `(program elpi:enforced true)` for a stored program, and `enforce_elpi(program, false)` SHALL retract it. At the commit of a transaction that turns enforcement on or changes the text of an enforced program or of one it includes, the full `tm.violation` check SHALL run, and any `error` violation SHALL fail the transaction with `ConstraintViolated`.

#### Scenario: Enforce over violating data
- **WHEN** two CEOs are stored and a program whose `tm.violation` finds them is enforced
- **THEN** the transaction fails with `ConstraintViolated` and nothing is written

#### Scenario: Unknown program
- **WHEN** enforcement is turned on for a node without `elpi:source`
- **THEN** the call fails with `UnknownProgram`

### Requirement: Delta checks at commit
Every other commit SHALL run each enforced program's `tm.violation-delta`, with `tm.added` and `tm.retracted` returning the transaction's own added and retracted statements. A delta violation SHALL fail the transaction with `ConstraintViolated`, unless the view as of the previous transaction already gave the same name and eids through `tm.violation error`. A program without `tm.violation-delta` clauses SHALL NOT be checked at commit.

#### Scenario: Second CEO refused
- **WHEN** the one-CEO program is enforced and a transaction asserts a second CEO
- **THEN** it fails with `ConstraintViolated` naming the constraint and both eids

#### Scenario: Unrelated write commits
- **WHEN** the program is enforced and a transaction asserts only names
- **THEN** it commits

#### Scenario: Retraction seeds a check
- **WHEN** a program's delta form reads `tm.retracted` for a project's owner and the only owner is retracted
- **THEN** the transaction fails with `ConstraintViolated`

### Requirement: ELPI constraint budget
The checks of one commit SHALL run under the constraint budget, with `max_seed_rows` as the cap on rows the builtins return. Running out of fuel or rows SHALL fail the transaction with `ConstraintBudgetExceeded`.

#### Scenario: Looping delta form
- **WHEN** an enforced program's delta form loops
- **THEN** the transaction fails with `ConstraintBudgetExceeded` and nothing is written
