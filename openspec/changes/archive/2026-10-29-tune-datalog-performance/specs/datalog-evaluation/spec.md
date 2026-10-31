## MODIFIED Requirements

### Requirement: Deterministic results
The rows of a run SHALL NOT depend on the backend, on parallel execution, on the probe-or-load choice or on whether interval sets that cover every instant are handled by a shortcut; they SHALL be returned sorted by the goal variables' values in the store's value order, then by interval start.

#### Scenario: Backends agree
- **WHEN** the same history is loaded into a SQLite file and a redb file and the same goal runs on both
- **THEN** both return identical rows in identical order

#### Scenario: Parallel and sequential agree
- **WHEN** a goal runs with `parallel_threshold = 0` and with the default
- **THEN** the rows are identical

#### Scenario: Mixed values sort in value order
- **WHEN** a goal's rows bind IRIs, strings, integers, doubles, booleans, dates and invented nodes in an order unrelated to their ids
- **THEN** the rows come back sorted by the store's value order of each column in turn, then by interval start, exactly as a sort that compares the decoded values gives

#### Scenario: Timeless and timed runs agree
- **WHEN** the same Horn program runs over a history whose statements are all valid always, once with `time: atemporal` and once over valid time, with and without `proof: true`
- **THEN** the four runs return the same tuples, each valid always, in the same order

### Requirement: Explain a program run
`View::explain_datalog(program, goal, &DatalogOptions)` SHALL run the program under the view's budget and return, per stratum and rule variant, the join order, each stored atom's access (probe or load and the permutation), estimated and actual rows, iterations, whether demand rewriting applied or why it fell back, and the wall-clock milliseconds spent evaluating the program and spent building the result rows.

#### Scenario: Explain shows access choice
- **WHEN** the reporting program is explained for `?- reports(ex:alice, X).`
- **THEN** the plan lists `ex:manager` with its access kind and the iteration count of the recursive stratum

#### Scenario: Explain shows phase times
- **WHEN** the reporting program is explained for `?- reports(ex:alice, X).`
- **THEN** the plan ends with one line giving the evaluation time and one giving the result time, each a non-negative number of milliseconds
