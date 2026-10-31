# elpi-programs Specification

## Purpose
TBD - created by archiving change add-elpi-programs. Update Purpose after archive.

## Requirements

### Requirement: Store an ELPI program
`TxElpi::store_elpi_program(program, text)` SHALL load the text (with its includes) to check it, then make `(program elpi:source "<text>")` the one live source of the node. A live source with other text SHALL be superseded, and equal text SHALL write nothing. It SHALL return the live statement's eid.

#### Scenario: Store and supersede
- **WHEN** a program is stored and then stored again with other text
- **THEN** the second call supersedes the first statement, and a view as of the first transaction runs the first text

#### Scenario: Invalid text writes nothing
- **WHEN** the text does not parse
- **THEN** the call fails with `ElpiProgram` and nothing is written

### Requirement: Rule language conflict
Storing ELPI text on a node with a live `n3:source` or `dl:source` SHALL fail with `RuleLanguageConflict`. Storing N3 rules or a Datalog program on a node with a live `elpi:source` SHALL fail the same way.

#### Scenario: Datalog node refused
- **WHEN** a node holds a Datalog program and ELPI text is stored on it
- **THEN** the call fails with `RuleLanguageConflict`

### Requirement: Includes
A line `accumulate <iri>.` SHALL include the live source of stored program `iri` from the same view. Includes SHALL nest. A cycle SHALL fail with `ProgramCycle` and a missing program with `UnknownProgram`.

#### Scenario: Library included
- **WHEN** program `lib` defines `ancestor` and program `app` has `accumulate <lib>.` and uses it
- **THEN** a goal run against `app` sees `ancestor`

#### Scenario: Cycle
- **WHEN** `a` includes `b` and `b` includes `a`
- **THEN** storing or running `a` fails with `ProgramCycle`

### Requirement: Program information
`View::elpi_program_info(program)` SHALL return the program's text, its source eid, the closure of its includes (each with its source eid), its monotonicity label and whether it is enforced.

#### Scenario: Closure listed
- **WHEN** `app` includes `lib`
- **THEN** the information for `app` lists `lib` with `lib`'s source eid
