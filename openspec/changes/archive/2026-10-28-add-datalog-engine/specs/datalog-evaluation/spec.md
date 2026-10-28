## Purpose

How a Datalog program runs over a view: goal queries and full reasoning, the scope it reads, goal-directed rewriting, provenance, limits, budgets, determinism and explain.

## ADDED Requirements

### Requirement: Goal queries over a view
With the facade feature `datalog`, `View::datalog(program, goal, &DatalogOptions)` SHALL evaluate the program over the view and return one row per distinct binding of the goal's variables, each with its valid interval set, plus the view's transaction number, `now`, the strata run, the loaded and probed predicates and a completion status. `program` SHALL be text or a stored program; `goal` SHALL be a `?-` body, or `None` to use the goals in the text. It SHALL write nothing.

#### Scenario: Reporting chain
- **WHEN** `ex:manager` holds `(alice, bob)` and `(bob, carol)`, the program defines `reports` as transitive `ex:manager`, and the goal is `?- reports(ex:alice, X).`
- **THEN** the rows are `X = ex:bob` and `X = ex:carol` and the store is unchanged

#### Scenario: Goal in the text
- **WHEN** the same program text ends with `?- reports(ex:alice, X).` and `goal` is `None`
- **THEN** the same two rows are returned

### Requirement: Full reasoning over a view
`View::datalog_reason(program, &DatalogOptions)` SHALL return every derived fact of the program's relations that are stored-predicate IRIs, which are not among the loaded statements, as conclusions `{ s, p, o, valid, proof }` in the shape of N3's conclusions. Derived relations with plain names SHALL be reported only when `include_relations` is set.

#### Scenario: Conclusions as statements
- **WHEN** the rule is `ex:worksIn(P, L) :- ex:worksAt(P, C), ex:locatedIn(C, L).` over `(alice worksAt acme)` and `(acme locatedIn berlin)`
- **THEN** the only conclusion is `(ex:alice ex:worksIn ex:berlin)`

### Requirement: Scope and pruning
A run SHALL read the statements of its scope: the whole view (default), the members of given named graphs, or the statements within `hops` of seed nodes. Within the scope it SHALL read only the predicates that some body atom names, unless a body uses `triple` with a variable predicate. The result SHALL report the predicates read and whether pruning applied.

#### Scenario: Unused predicates are not read
- **WHEN** the view holds 100 `ex:name` statements and the program reads only `ex:manager`
- **THEN** the result reports `ex:manager` as the only predicate read and pruning as applied

#### Scenario: Graph scope
- **WHEN** `(ex:a ex:manager ex:b)` is a member of `ex:g1`, `(ex:b ex:manager ex:c)` is not, and the scope is `Graphs([ex:g1])`
- **THEN** `?- reports(ex:a, X).` returns `X = ex:b` only

### Requirement: Magic sets preserve answers
When a goal binds a constant, evaluation SHALL be restricted to the facts relevant to that goal, and the rows SHALL equal those of evaluating the whole program and then filtering by the goal. `DatalogOptions::demand = false` SHALL force whole-program evaluation.

#### Scenario: Goal-directed equals full
- **WHEN** a random graph and the `reports` program are evaluated for `?- reports(c, X).` with demand on and off
- **THEN** both return the same rows, and with demand on the explain shows fewer derived facts

### Requirement: Provenance on request
With `proof: true`, each row and conclusion SHALL carry one derivation: the rule index and its premises, each premise either a statement eid or a reference to another derived fact. Without it, rows SHALL carry no proof. The rows SHALL be the same with and without proofs.

#### Scenario: Proof of a chain
- **WHEN** `?- reports(ex:alice, ex:carol).` runs with `proof: true` over e1 = `(alice manager bob)` and e2 = `(bob manager carol)`
- **THEN** the row's proof uses the recursive rule with premises a derived `reports(alice, bob)` (whose proof cites e1) and e2

### Requirement: Limits and completion
`DatalogOptions::limits` SHALL bound iterations, derived facts, interval fragments and new values. Reaching a limit SHALL stop the run with status `Incomplete { limits }`, and every reported row SHALL still follow from the program; `require_complete` SHALL turn it into the error `DatalogIncomplete`.

#### Scenario: Derived-fact limit
- **WHEN** the transitive closure of a 1 000-node chain runs with `max_facts = 100`
- **THEN** the status is `Incomplete` with limit `Facts` and every reported row is a true `reports` fact

### Requirement: Budgets and cancellation
A run SHALL respect the view's `QueryBudget`: the deadline and cancellation token SHALL stop it with the budget's typed error, and row and byte limits SHALL count the rows returned.

#### Scenario: Deadline stops a closure
- **WHEN** a closure over 10⁶ edges runs under a 1 ms deadline
- **THEN** it fails with the budget's deadline error and returns no rows

### Requirement: Deterministic results
The rows of a run SHALL NOT depend on the backend, on parallel execution or on the probe-or-load choice; they SHALL be returned sorted by the goal variables' values in the store's value order, then by interval start.

#### Scenario: Backends agree
- **WHEN** the same history is loaded into a SQLite file and a redb file and the same goal runs on both
- **THEN** both return identical rows in identical order

#### Scenario: Parallel and sequential agree
- **WHEN** a goal runs with `parallel_threshold = 0` and with the default
- **THEN** the rows are identical

### Requirement: Explain a program run
`View::explain_datalog(program, goal, &DatalogOptions)` SHALL run the program under the view's budget and return, per stratum and rule variant, the join order, each stored atom's access (probe or load and the permutation), estimated and actual rows, iterations, and whether demand rewriting applied or why it fell back.

#### Scenario: Explain shows access choice
- **WHEN** the reporting program is explained for `?- reports(ex:alice, X).`
- **THEN** the plan lists `ex:manager` with its access kind and the iteration count of the recursive stratum

### Requirement: Agreement with N3 on Horn programs
For a program of positive rules without operators, aggregates, invention or builtins other than comparison, and the equivalent N3 rule set, `datalog_reason` and N3 `reason` over the same view SHALL return the same conclusions with the same valid intervals.

#### Scenario: Horn differential
- **WHEN** a random history with random valid intervals is reasoned over with the `worksIn` rule in Datalog and in N3
- **THEN** the conclusions and their intervals are equal
