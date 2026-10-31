## ADDED Requirements

### Requirement: Goal queries over a view
`View::elpi(program, goal, opts)` SHALL load the program after the tiramemsu prelude, solve `goal` with the host builtins reading the view, and return one row per solution with each named variable's value. It SHALL write nothing. Parse and type errors SHALL fail with `ElpiProgram`, runtime errors with `ElpiRun`, and an exhausted fuel or row budget with `ElpiOutOfFuel`.

#### Scenario: Transitive closure
- **WHEN** `alice manager bob` and `bob manager carol` are stored, and the program defines `reports X Y` through `tm.triple X (iri "urn:tiramemsu:v:manager") Y _` and recursion
- **THEN** the goal `reports (iri "urn:tiramemsu:v:alice") Y` returns the rows `Y = bob` and `Y = carol`

#### Scenario: Syntax error
- **WHEN** the program text does not parse
- **THEN** the call fails with `ElpiProgram` and nothing is read

#### Scenario: Fuel runs out
- **WHEN** a goal loops forever and the fuel is 10⁴
- **THEN** the call fails with `ElpiOutOfFuel`

### Requirement: Terms cross both ways
IRIs, strings, integers, doubles, booleans, language-tagged strings, other literals, nodes, blank nodes, transactions and statements SHALL be passed to programs as the prelude's constructors. A program's solution SHALL decode back to the same value. A term that is not a value (a variable, a lambda, a list) SHALL come back as `ElpiTerm::Other` with its printed form.

#### Scenario: Native integers
- **WHEN** `(ex:a ex:age 41)` is stored and the goal is `tm.triple _ (iri "…age") (num N) _, M is N + 1`
- **THEN** `M` decodes to `Value::Int(42)`

#### Scenario: Bound arguments are lookups
- **WHEN** the subject argument is bound to an IRI that is not in the dictionary
- **THEN** `tm.triple` has no rows and the store is not scanned

### Requirement: Valid time of a run
`query` and `check` SHALL read the statements valid at one instant: the view's valid-at instant when it has one, else `opts.valid_at`, else the clock. Transaction time SHALL be the view's.

#### Scenario: A closed fact is not read
- **WHEN** `(ex:a ex:role ex:CEO)` is valid `[2020, 2024)` and the query runs at 2025
- **THEN** `tm.triple` does not return it

### Requirement: Conclusions carry stated support
`conclude` SHALL collect every solution of `tm.conclude (triple S P O) Support`. Each conclusion SHALL carry its support eids and the intersection of their valid intervals, and a conclusion whose intersection is empty SHALL be dropped. An eid the run's builtins never returned, or an empty support list, SHALL fail the run with `ElpiRun`. A program the monotonicity scan marks `non_monotonic` SHALL be evaluated per valid-time segment.

#### Scenario: Support is the premises
- **WHEN** the program concludes `(triple X (iri "…grandparent") Z)` from two `parent` statements it read as `E1` and `E2`, with `Support = [E1, E2]`
- **THEN** the conclusion carries exactly those two eids

#### Scenario: Forged support
- **WHEN** a program lists `(eid 999)` that no builtin returned
- **THEN** the run fails with `ElpiRun`

#### Scenario: Intervals intersect
- **WHEN** the premises are valid `[2020, 2025)` and `[2022, ∞)`
- **THEN** the conclusion is valid `[2022, 2025)`

#### Scenario: Negation is labelled
- **WHEN** the program uses `not`
- **THEN** the result marks it `non_monotonic`

### Requirement: Full constraint check
`View::elpi_check(program, opts)` SHALL collect every solution of `tm.violation Severity Name Msg Eids` and report each violation with its severity, name, message, eids and the intersection of the eids' intervals.

#### Scenario: Two CEOs
- **WHEN** two statements give two different subjects `ex:role ex:CEO`, and the program has `tm.violation error "one_ceo" "two CEOs" [E1, E2]` for them
- **THEN** the check reports one `error` violation `one_ceo` with both eids

### Requirement: Optional ELPI feature
The facade SHALL expose ELPI only behind the cargo feature `elpi`, off by default, and the CLI SHALL NOT enable it by default. `tm-elpi` SHALL forbid unsafe code and depend only on registry crates (once lean-elpi 0.4 is published).

#### Scenario: Core tier unchanged
- **WHEN** the facade builds without the `elpi` feature
- **THEN** `leanelpi` is not in its dependency tree
