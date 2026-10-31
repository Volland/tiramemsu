# elpi-materialization Specification

## Purpose
TBD - created by archiving change add-elpi-materialization. Update Purpose after archive.

## Requirements

### Requirement: Materialize an ELPI program
`Db::materialize_elpi(program, opts)` SHALL derive each conclusion of the stored program's `tm.conclude` as a statement. Its support SHALL be the stated eids, plus the program's `elpi:source` statement, plus the `elpi:source` statement of every included program. Its valid interval SHALL be the one `conclude` computed. The method SHALL return the derived, existing, widened and reconciled eids. Deriving the same conclusion from the same support again SHALL write nothing.

#### Scenario: Grandparents
- **WHEN** `a parent b` and `b parent c` are stored and the program concludes `grandparent`
- **THEN** `(a grandparent c)` is derived with three supports: the two premises and the program's source

#### Scenario: Idempotent
- **WHEN** the program is materialized twice with no writes between
- **THEN** the second run derives nothing and lists the conclusion as existing

### Requirement: Conclusions fall with premises and programs
Retracting a premise, or superseding the program's text or the text of an included program, SHALL retract the conclusions that rest on it through the support cascade.

#### Scenario: Premise retracted
- **WHEN** `b parent c` is retracted
- **THEN** `(a grandparent c)` is no longer live

#### Scenario: Library edited
- **WHEN** the included library's text is superseded
- **THEN** the conclusions of the program that includes it are no longer live

### Requirement: Reconcile every run
After writing, a run SHALL retract the program's live conclusions that it did not derive again, and SHALL label the transaction `(tx elpi:materialized program)`. A `non_monotonic` program SHALL be listed in the report's `needs_rerun_after_writes`.

#### Scenario: Negation reconciled
- **WHEN** a program concludes `(x unmanaged true)` while `x` has no manager, is materialized, then a manager is asserted and the program is materialized again
- **THEN** the second run retracts `(x unmanaged true)` and the report lists it under `reconciled`

### Requirement: Own conclusions are not input
A materialization run SHALL NOT read the program's own live conclusions, or the statements about them.

#### Scenario: No self-support
- **WHEN** the program's rules would read its own previous conclusions
- **THEN** the run's builtins do not return them
