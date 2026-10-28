## Purpose

Datalog over valid time: what a derived fact means at each instant, the DatalogMTL metric operators and Allen relations, windows for recursion through time, negation and aggregates over time, and the pinned transaction clock.

## ADDED Requirements

### Requirement: Pointwise valid time with intervals
A derived fact SHALL hold at instant `t` if and only if it follows from the statements of the scope valid at `t`. Each row and conclusion SHALL report its valid time as a set of disjoint, non-adjacent half-open intervals over integer epoch milliseconds, with unbounded ends as such. A `valid_at(t)` view SHALL give intervals `[t, t+1)`.

#### Scenario: Facts never simultaneously valid do not combine
- **WHEN** `(alice worksAt acme)` is valid `[2020, 2022)`, `(acme locatedIn berlin)` is valid `[2024, ∞)` and the `worksIn` rule runs
- **THEN** there is no conclusion

#### Scenario: Overlap gives the intersection
- **WHEN** the same facts are valid `[2020, 2025)` and `[2024, ∞)`
- **THEN** the only conclusion is `(alice worksIn berlin)` valid `[2024, 2025)`

#### Scenario: Two derivations coalesce
- **WHEN** `(alice worksIn berlin)` follows from one pair of facts over `[2020, 2023)` and from another over `[2023, 2026)`
- **THEN** it is reported once, valid `[2020, 2026)`

#### Scenario: Instant-by-instant oracle
- **WHEN** random facts with random intervals are evaluated with a random positive program
- **THEN** at each tested instant `t`, a fact is reported as valid at `t` exactly when the brute-force evaluation of the program over the statements valid at `t` derives it

### Requirement: Metric temporal operators
Bodies SHALL support `past[a, b] A` (A held at some instant in `[t−b, t−a]`), `always_past[a, b] A` (A held throughout `[t−b, t−a]`), `future[a, b] A`, `always_future[a, b] A`, `A since[a, b] B` and `A until[a, b] B`, with `0 ≤ a ≤ b` durations and `b` possibly `inf`, under DatalogMTL's semantics over the integer timeline.

#### Scenario: Fever within the last three days
- **WHEN** `(ex:p ex:fever true)` is valid `[day 10, day 11)` and the rule is `alert(X) :- past[0d, 3d] ex:fever(X, true).`
- **THEN** `alert(ex:p)` is valid `[day 10, day 14)`

#### Scenario: Always in the past
- **WHEN** `(ex:p ex:employed true)` is valid `[day 0, day 100)` and the rule is `tenured(X) :- always_past[0d, 30d] ex:employed(X, true).`
- **THEN** `tenured(ex:p)` is valid `[day 30, day 100)`

#### Scenario: Since
- **WHEN** `(s1 ex:alarm true)` is valid `[5, 6)` and `(s1 ex:unacked true)` is valid `[5, 20)`, and the rule is `pending(S) :- ex:unacked(S, true) since[0, inf] ex:alarm(S, true).`
- **THEN** `pending(s1)` is valid `[5, 20)`

### Requirement: Always operators in heads
A head MAY be `always_past[a, b] H` or `always_future[a, b] H`, meaning H holds throughout the window relative to each instant at which the body holds. `past` and `future` (sometime) in a head SHALL fail analysis with `DatalogNotHorn`.

#### Scenario: Holds for the next week
- **WHEN** `ex:booked(r1, true)` holds at `[day 1, day 2)` and the rule is `always_future[0d, 7d] blocked(R) :- ex:booked(R, true).`
- **THEN** `blocked(r1)` is valid `[day 1, day 9)`

#### Scenario: Sometime in a head is refused
- **WHEN** a rule's head is `future[0d, 1d] p(X)`
- **THEN** analysis fails with `DatalogNotHorn`

### Requirement: Allen relations
The 13 Allen relations SHALL be builtins over statement eids or interval pairs (`before(E1, E2)`, `meets`, `overlaps`, `starts`, `during`, `finishes`, `equals` and the six converses), with `interval(E, I)`, `start(I, T)`, `end(I, T)` and `duration(I, D)`, evaluated with the same definitions SPARQL and Cypher use, unbounded ends included.

#### Scenario: Job meets the next job
- **WHEN** e1 is valid `[2020, 2022)` and e2 is valid `[2022, 2025)`, and a goal binds them through `stmt(E1, …)` and `stmt(E2, …)`
- **THEN** `meets(E1, E2)` is true and `overlaps(E1, E2)` is false, while the same goal through the facts `p(S, O) @ E` has no row, because the two are never valid at one instant

### Requirement: Windows and recursion through time
A program that analysis reports as recursive through a metric operator SHALL require `DatalogOptions::window`; without one it SHALL fail with `DatalogUnboundedRecursion`. With a window, intervals SHALL be clipped to it before each iteration and the run SHALL terminate. Other programs SHALL run without a window, and a given window SHALL clip their results.

#### Scenario: Contagion needs a window
- **WHEN** the recursive `contagious` program runs without a window
- **THEN** it fails with `DatalogUnboundedRecursion`

#### Scenario: Contagion within a window
- **WHEN** it runs with window `[day 0, day 30)` over a contact chain `a → b → c` with `a` sick at day 1 and contacts at days 2 and 3
- **THEN** `contagious` holds for `a`, `b` and `c` with intervals inside the window

### Requirement: Negation over time
`not A` SHALL hold at `t` for a binding when A does not hold at `t` for that binding; the result interval set SHALL be the complement of A's within the binding's positive interval set.

#### Scenario: Unmanaged until a manager arrives
- **WHEN** `(bob type Employee)` is valid always, `(bob manager carol)` is valid `[2023, ∞)` and the `unmanaged` rule runs
- **THEN** `unmanaged(bob)` is valid `(−∞, 2023)` only

### Requirement: Aggregates over time
An aggregate SHALL be evaluated at each instant over the facts valid then: its result for a group SHALL be a piecewise-constant interval set, one value per maximal interval with the same contributing facts.

#### Scenario: Headcount changes with hires
- **WHEN** `(acme type Company)` and `(acme employs a)` are valid `[2020, ∞)`, `(acme employs b)` is valid `[2022, 2024)`, and the rule is `headcount(C, N) :- rdf:type(C, ex:Company), N = count : { ex:employs(C, _) }.`
- **THEN** `headcount(acme, 1)` is valid `[2020, 2022)` and `[2024, ∞)` and `headcount(acme, 2)` is valid `[2022, 2024)`

### Requirement: Transaction time and the pinned clock
A run SHALL read the statements visible in the view's transaction time. `now()` SHALL be the view's instant (the as-of instant, or one instant taken when a run on a now view starts), constant for the run and reported in the result. Metric operators SHALL range over valid time only.

#### Scenario: As-of ignores later facts
- **WHEN** `(alice manager bob)` is asserted in transaction 2 and the goal runs on `as_of(Tx(1))`
- **THEN** there are no rows

#### Scenario: Reproducible now
- **WHEN** `?- T = now().` runs twice on `as_of(Tx(3))`
- **THEN** both return the instant of transaction 3

### Requirement: Atemporal mode
`DatalogOptions::time = Atemporal` SHALL ignore valid time: every statement of the scope SHALL count as valid always, every result SHALL be unbounded, and metric operators SHALL fail analysis with `DatalogTemporalOperatorInAtemporal`.

#### Scenario: Atemporal combines disjoint facts
- **WHEN** the facts of "Facts never simultaneously valid do not combine" are reasoned over atemporally
- **THEN** `(alice worksIn berlin)` is concluded, unbounded
