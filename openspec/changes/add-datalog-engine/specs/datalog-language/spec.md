## Purpose

The Datalog program text: how rules, goals and constraints are written, how atoms map to stored statements, which builtins exist, and which programs are refused before they run.

## ADDED Requirements

### Requirement: Program text
A program SHALL be UTF-8 text of `@prefix p: <iri> .` declarations, rules `head :- body.`, facts `head.`, goals `?- body.` and constraints `constraint name warn|error :- body.`, with `%` line comments. Constants SHALL be prefixed names, `<iri>`s, string, number and boolean literals, and typed literals `"lex"^^dt`. Variables SHALL start with an uppercase letter or `_`; `_` alone SHALL be anonymous. A syntax error SHALL fail with `DatalogParse { line, column, message }`.

#### Scenario: Rules goal and constraint parse
- **WHEN** the text declares `@prefix ex: <http://example.org/> .`, the rules `reports(X, Y) :- ex:manager(X, Y).` and `reports(X, Z) :- reports(X, Y), ex:manager(Y, Z).`, the goal `?- reports(ex:alice, X).` and `constraint no_cycle error :- reports(X, X).`
- **THEN** it parses into two rules, one goal and one `error` constraint named `no_cycle`

#### Scenario: Syntax error has a position
- **WHEN** the text is `reports(X, Y) :- ex:manager(X, Y)` with no final period
- **THEN** parsing fails with `DatalogParse` naming line 1 and the column of the end of input

### Requirement: Stored atoms
An atom whose predicate is an IRI and that has two arguments, `p(S, O)`, SHALL match the live statements of the view with predicate `p`, under set semantics on `(S, O)`. The form `p(S, O) @ E` SHALL also bind `E` to the statement's eid, with one binding per statement. A predicate IRI that the program also defines in a rule head SHALL be a derived relation, not a stored one; a derived relation SHALL have a fixed arity, and any arity SHALL be allowed for derived relations with plain names.

#### Scenario: Parallel statements under set semantics
- **WHEN** two live statements `(ex:a ex:knows ex:b)` exist and the goal is `?- ex:knows(ex:a, Y).`
- **THEN** there is one row `Y = ex:b`

#### Scenario: Eid binding keeps parallel statements apart
- **WHEN** the same two statements exist and the goal is `?- ex:knows(ex:a, Y) @ E.`
- **THEN** there are two rows, one per eid

#### Scenario: Layers are ordinary atoms
- **WHEN** e1 = `(ex:alice ex:worksAt ex:acme)` has the layer `(e1 ex:confidence 0.9)` and the goal is `?- ex:worksAt(X, C) @ E, ex:confidence(E, K), K > 0.8.`
- **THEN** there is one row `X = ex:alice, C = ex:acme, E = e1, K = 0.9`

### Requirement: Reserved atoms
The reserved atoms SHALL be `triple(S, P, O)` (any stored predicate, P may be a variable), `stmt(E, S, P, O)`, `valid(E, From, To)` (the statement's valid interval, with an unbounded end as the constant `inf`), `in_graph(E, G)` (named-graph membership), `added(E, T)` (the transaction that inserted the statement) and `tx_instant(T, Ms)` (a transaction's instant in epoch milliseconds). Their names SHALL NOT be used as rule heads; doing so SHALL fail with `DatalogReserved`.

#### Scenario: Valid interval as data
- **WHEN** e1 = `(ex:alice ex:worksAt ex:acme)` is valid `[2020-01-01, 2022-01-01)` and the goal is `?- ex:worksAt(ex:alice, C) @ E, valid(E, F, T).`
- **THEN** the row binds `F` and `T` to those two instants

#### Scenario: Reserved head refused
- **WHEN** a rule's head is `valid(X, Y, Z)`
- **THEN** analysis fails with `DatalogReserved`

### Requirement: Builtins
Bodies SHALL support comparisons (`=`, `!=`, `<`, `<=`, `>`, `>=`) under the store's value order, arithmetic (`+`, `-`, `*`, `/`, `%`) on numbers and on instants and durations (`30d`, `12h`, `5m`, `10s`, `100ms`), string builtins (`concat`, `strlen`, `contains`, `starts_with`, `lower`, `upper`), and `now()`. A builtin over unbound variables SHALL fail analysis with `DatalogUnsafe`. A builtin applied to values of the wrong kind SHALL be false for that binding, not an error.

#### Scenario: Duration arithmetic
- **WHEN** `ex:due(ex:task1, D)` holds with `D = 2026-01-10T00:00:00Z` and the goal is `?- ex:due(T, D), E = D + 30d.`
- **THEN** `E` is `2026-02-09T00:00:00Z`

#### Scenario: Wrong kind is false
- **WHEN** the goal is `?- ex:name(X, N), N > 3.` and every name is a string
- **THEN** there are no rows and no error

### Requirement: Stratified negation and aggregates
`not A` and the aggregates `count`, `sum`, `min` and `max` (written `R = count : { body }` or `R = sum V : { body }`) SHALL be allowed when the program can be stratified: no relation depends on itself through negation or aggregation. A program that cannot be stratified SHALL fail analysis with `DatalogUnstratifiable` naming one cycle.

#### Scenario: Unmanaged employees
- **WHEN** `ex:Employee` members are `ex:bob` and `ex:carol`, only `ex:carol` has an `ex:manager`, and the rule is `unmanaged(X) :- rdf:type(X, ex:Employee), not ex:manager(X, _).`
- **THEN** `unmanaged` holds for `ex:bob` only

#### Scenario: Negative cycle refused
- **WHEN** the program is `p(X) :- q(X), not r(X).` and `r(X) :- q(X), not p(X).`
- **THEN** analysis fails with `DatalogUnstratifiable` naming `p` and `r`

### Requirement: Safety
Every variable of a head, of a negated atom, of a comparison or of a builtin SHALL occur in a positive atom of the same body, or be bound by an `=` from bound variables, except a head variable introduced by `exists`. An unsafe rule SHALL fail analysis with `DatalogUnsafe` naming the rule and the variable.

#### Scenario: Unbound head variable
- **WHEN** the rule is `p(X, Y) :- q(X).`
- **THEN** analysis fails with `DatalogUnsafe` naming `Y`

### Requirement: Value invention
A head MAY introduce a variable with `exists Y:`; each firing SHALL bind it to a node determined by the rule and the values of the head's other variables, so the same firing in another run yields the same node identity within that run's results. Heads MAY compute values with builtins. A program in which a relation depends on itself through an `exists` variable or a computed head value SHALL be refused with `DatalogUnboundedInvention` unless `limits.max_new_values` is set; with the limit, reaching it SHALL stop the run with status `Incomplete` and limit `NewValues`.

#### Scenario: Every employee has a manager
- **WHEN** the rule is `exists M: ex:manager(X, M) :- rdf:type(X, ex:Employee), not ex:manager(X, _).` and `ex:bob` has no manager
- **THEN** one conclusion `ex:manager(ex:bob, M1)` is reported with `M1` an invented node

#### Scenario: Recursive invention needs a limit
- **WHEN** the program is `n(0).` and `n(Y) :- n(X), Y = X + 1.` with no `max_new_values`
- **THEN** analysis fails with `DatalogUnboundedInvention`

#### Scenario: Invention fuse
- **WHEN** the same program runs with `max_new_values = 100`
- **THEN** the status is `Incomplete` with limit `NewValues` and at most 100 values of `n` are reported

### Requirement: Termination analysis
Before evaluation, analysis SHALL report for the program: its strata, whether each stratum is monotonic, the stored predicates its bodies use, whether a body uses a variable predicate, whether any relation depends on itself through a metric temporal operator, and whether any relation depends on itself through value invention. A program that passes analysis and runs within its window and limits SHALL terminate.

#### Scenario: Analysis of a recursive temporal program
- **WHEN** the program is `contagious(X) :- ex:sick(X).` and `contagious(Y) :- past[0d, 3d] contagious(X), ex:contact(X, Y).`
- **THEN** analysis reports one recursive stratum, monotonic, using `ex:sick` and `ex:contact`, with recursion through a metric operator
