## Purpose

Integrity constraints written in Datalog: how they are declared and how violations are checked and reported on any view. Enforcement at commit is added by `add-datalog-constraints`.

## ADDED Requirements

### Requirement: Declare constraints
A constraint SHALL be declared as `constraint name warn|error :- body.` with a name unique within its program. Its body SHALL follow the rules for bodies (safety, stratification, operators). A duplicate name SHALL fail analysis with `DatalogDuplicateConstraint`.

#### Scenario: Two constraints
- **WHEN** a program declares `constraint one_ceo error :- ex:role(X, ex:CEO), ex:role(Y, ex:CEO), X != Y.` and `constraint owned warn :- rdf:type(P, ex:Project), not ex:owner(P, _).`
- **THEN** `program_info` lists `one_ceo` as `error` and `owned` as `warn`

#### Scenario: Duplicate name
- **WHEN** two constraints are named `one_ceo`
- **THEN** analysis fails with `DatalogDuplicateConstraint`

### Requirement: Check constraints
`View::check(program, &CheckOptions)` SHALL evaluate every constraint of the program (or those named in `only`) over the view and return, per violated constraint, its name, severity and violations; each violation SHALL list the body's variable bindings, the eids of the statements its proof used and the valid interval set over which it holds. It SHALL write nothing. Constraints over time SHALL be checked pointwise.

#### Scenario: Two CEOs at once
- **WHEN** `(ex:a ex:role ex:CEO)` is valid `[2020, 2024)` and `(ex:b ex:role ex:CEO)` is valid `[2023, ∞)`
- **THEN** `check` reports `one_ceo` violated for `{X: a, Y: b}` and `{X: b, Y: a}`, each valid `[2023, 2024)`, citing both eids

#### Scenario: Successive CEOs are fine
- **WHEN** the second CEO is valid `[2024, ∞)`
- **THEN** `check` reports no violation of `one_ceo`

#### Scenario: Check as of the past
- **WHEN** the second CEO statement was asserted in transaction 7 and `check` runs on `as_of(Tx(6))`
- **THEN** no violation of `one_ceo` is reported
