## Purpose

Defines the logical query representation that SPARQL, Cypher and the programmatic API all lower to. It covers the operators, the per-pattern time view, the semantic flags that encode dialect differences, eid binding, parameters, and constant handling at plan time, so that one executor can answer both dialects exactly.

## ADDED Requirements

### Requirement: Logical operator set
The query IR SHALL represent every query as a tree built from exactly these operators: TriplePattern, PathPattern, Values, Join, LeftJoin, Filter, Union, Extend, Aggregate, Project and OrderLimit. The executor SHALL accept any structurally valid tree of these operators, nested to any depth, and SHALL return a result whose columns are the variables the root operator exposes.

#### Scenario: Basic graph pattern as a join of triple patterns
- **WHEN** an IR `Join[TriplePattern(?a, v:worksAt, ?c), TriplePattern(?c, v:name, ?n)]` is executed on a store that holds `(alice worksAt acme)` and `(acme name "Acme Corp")`
- **THEN** the result contains exactly one row binding `?a = alice`, `?c = acme`, `?n = "Acme Corp"`

#### Scenario: Nested composition
- **WHEN** an IR `Project[?a](Filter(?n != "x", LeftJoin(Join[…], Union[…])))` is executed
- **THEN** the executor evaluates every operator according to its definition and returns only column `?a`

#### Scenario: Empty join is the unit relation
- **WHEN** an IR `Extend(?x := 1, Join[])` is executed
- **THEN** the result has exactly one row, with `?x = 1`

#### Scenario: Values provides inline bindings
- **WHEN** an IR `Join[Values(?p: [v:alice, v:bob]), TriplePattern(?p, v:age, ?age)]` is executed and only alice has an age
- **THEN** the result has one row, for alice

#### Scenario: Values with an undefined cell
- **WHEN** a Values row leaves variable `?y` undefined
- **THEN** that row leaves `?y` missing in its solution, and does not bind it to any term

### Requirement: Result column order
The result columns SHALL be the root `Project`'s variables in their declared order. When the root is not a `Project`, the columns SHALL be every variable the root exposes, in order of first binding in a left-to-right depth-first walk of the tree. Column order SHALL be deterministic for a given IR.

#### Scenario: Project fixes column order
- **WHEN** an IR whose root is `Project[?n, ?a](…)` is executed
- **THEN** the result's column list is exactly `[?n, ?a]`

#### Scenario: Implicit column order
- **WHEN** an IR `Join[TriplePattern(?a, v:p, ?b), TriplePattern(?b, v:q, ?c)]` without a Project is executed
- **THEN** the result's column list is `[?a, ?b, ?c]`

### Requirement: Per-pattern view
Every TriplePattern and PathPattern SHALL carry its own View, made of a transaction-time selector (`Now`, `AsOf(TimeRef)` or `History`) and a valid-time selector (`Unfiltered` or `At(epoch_ms)`). The executor SHALL evaluate each pattern under its own View only, and SHALL NOT apply the View of the handle the query was run from to a pattern that carries a different View.

#### Scenario: Two patterns under different views in one query
- **WHEN** an IR joins `TriplePattern(v:alice, v:worksAt, ?before)` with View `AsOf(Tx(150))` and `TriplePattern(v:alice, v:worksAt, ?after)` with View `Now`, and alice's employer was superseded after tx 150
- **THEN** the single result row binds `?before` to the old employer and `?after` to the new one

#### Scenario: The handle view does not override explicit pattern views
- **WHEN** an IR whose only pattern carries View `Now` is executed through a handle obtained from `as_of(Tx(10))`
- **THEN** the pattern is evaluated under `Now`

#### Scenario: Handle view as the lowering default
- **WHEN** a caller asks a handle created by `db.as_of(Tx(10)).valid_at(d)` for its view descriptor
- **THEN** it receives View `{tx: AsOf(Tx(10)), valid: At(d)}`, which front ends use as the default View for patterns that carry no time clause

#### Scenario: Overlaying one part of a view
- **WHEN** a query-level clause gives only a transaction-time selector `AsOf(Tx(150))` over a default View `{tx: Now, valid: At(d)}`
- **THEN** the resulting pattern View is `{tx: AsOf(Tx(150)), valid: At(d)}`

### Requirement: Time references resolved at plan time
An `AsOf(Instant(ms))` selector SHALL be resolved to the largest transaction number whose instant is ≤ `ms`, inside the same read snapshot that executes the query. An `AsOf` that resolves to a point before the first transaction SHALL make the pattern empty. `AsOf(Tx(t))` SHALL be used as given.

#### Scenario: Instant resolves to a transaction
- **WHEN** transactions 1, 2 and 3 committed at instants 1000, 2000 and 3000, and a pattern carries `AsOf(Instant(2500))`
- **THEN** the pattern is evaluated as `AsOf(Tx(2))`

#### Scenario: Instant before the first transaction
- **WHEN** a pattern carries `AsOf(Instant(500))` and the first transaction committed at instant 1000
- **THEN** the pattern contributes no rows, and no SQL is executed for it

### Requirement: Semantic flags
Every IR query SHALL carry exactly one value for each of three semantic flags: `match_mode` (`Homomorphism` or `RelIsomorphism`), `missing` (`Unbound` or `Null3VL`) and `graph_set` (`SetOfTriples` or `BagOfEids`). The same operator tree executed under different flag values SHALL produce results that differ only as the flags define. The IR SHALL provide the presets SPARQL = (`Homomorphism`, `Unbound`, `SetOfTriples`) and Cypher = (`RelIsomorphism`, `Null3VL`, `BagOfEids`).

#### Scenario: Presets
- **WHEN** a caller builds a query with the SPARQL preset
- **THEN** its flags are `match_mode = Homomorphism`, `missing = Unbound`, `graph_set = SetOfTriples`

#### Scenario: Same tree, different graph_set
- **WHEN** two live eids carry the same `(alice, knows, bob)` and the tree `TriplePattern(?a, v:knows, ?b)` runs once under `SetOfTriples` and once under `BagOfEids`
- **THEN** the first run returns one row and the second returns two rows

### Requirement: Eid binding
A TriplePattern SHALL optionally bind the statement's eid to a variable. An eid variable SHALL be usable in the subject or object position of other patterns, which reaches layers, and in the eid slot of another pattern, which requires both patterns to match the same statement. The eid SHALL be returned as a statement value.

#### Scenario: Annotation on a statement
- **WHEN** an IR joins `TriplePattern(v:alice, v:worksAt, ?c, eid = ?r)` with `TriplePattern(?r, v:confidence, ?conf)`, and e1 = `(alice worksAt acme)` is annotated with `(e1 confidence 0.8)`
- **THEN** the result row binds `?r` to statement e1 and `?conf` to 0.8

#### Scenario: A belief referencing a statement
- **WHEN** an IR joins `TriplePattern(?a, v:worksAt, ?c, eid = ?r)` with `TriplePattern(?b, v:supportedBy, ?r)`
- **THEN** each row pairs a statement with the beliefs whose object is that statement's eid

#### Scenario: Same eid variable in two patterns
- **WHEN** two TriplePatterns share the eid variable `?r` but have different constant predicates
- **THEN** the result is empty, because one statement has only one predicate

#### Scenario: Eid equal to a constant
- **WHEN** an IR filters `?r = e1` over `TriplePattern(?s, ?p, ?o, eid = ?r)`
- **THEN** the result has exactly one row, with the parts of e1, if e1 is visible in the pattern's view

### Requirement: Constants encoded at plan time
Constants in patterns, Values and expressions SHALL be encoded to ObjectIds before any SQL is executed. Inline-encodable values (integers in the 60-bit range, booleans, dates, datetimes, strings of at most 7 UTF-8 bytes, statement, transaction and node ids) SHALL be encoded without consulting the term dictionary. All other values SHALL be looked up in the term dictionary, read-only. Planning SHALL never insert terms.

#### Scenario: Inline constant needs no dictionary
- **WHEN** a pattern has the constant object `42` (integer)
- **THEN** the planner encodes it as an `INT` ObjectId without reading the term table

#### Scenario: Dictionary constant is found
- **WHEN** a pattern has constant predicate `v:worksAt` and that IRI is in the term dictionary
- **THEN** the pattern compares the predicate column with that term's ObjectId

#### Scenario: Planning does not write
- **WHEN** a query whose constants are missing from the dictionary is executed
- **THEN** the term table and the `meta` counters are unchanged afterwards

### Requirement: Short-circuit on constants that cannot match
A TriplePattern SHALL be treated as empty, and SHALL NOT be sent to SQLite, when any of these holds: a constant in it is missing from the term dictionary; its predicate constant is not an IRI; its subject constant is a literal; or its View resolves to a point before the first transaction. Emptiness SHALL propagate as follows: Join, Filter, Extend, Project and OrderLimit over an empty input are empty; a Union drops empty branches and is empty only if every branch is empty; a LeftJoin with an empty right side returns its left side with the right-only variables missing; a LeftJoin with an empty left side is empty; an Aggregate with no grouping keys over an empty input returns exactly one row. When the whole query is empty, the executor SHALL return zero rows with the correct columns and SHALL execute no SQL.

#### Scenario: Unknown IRI constant
- **WHEN** a query contains `TriplePattern(?s, <urn:never-seen>, ?o)` joined with other patterns
- **THEN** the result is empty, and no SQL statement is executed

#### Scenario: Unknown long string constant
- **WHEN** a pattern has the object `"a string longer than seven bytes"` that was never stored
- **THEN** the pattern is empty

#### Scenario: Literal in predicate position
- **WHEN** a pattern's predicate constant is the integer `5`
- **THEN** the pattern is empty, and no error is raised

#### Scenario: Optional side missing from the dictionary
- **WHEN** an IR is `LeftJoin(TriplePattern(?p, v:name, ?n), TriplePattern(?p, <urn:unknown>, ?x))`
- **THEN** every person with a name is returned, with `?x` missing

#### Scenario: Union with one empty branch
- **WHEN** one branch of a Union contains an unknown IRI constant and the other branch matches two rows
- **THEN** the result has those two rows

#### Scenario: Count over an empty input
- **WHEN** an IR `Aggregate(group = [], count(*) as ?n)` is executed over a pattern with an unknown constant
- **THEN** the result has exactly one row, with `?n = 0`

#### Scenario: Grouped aggregate over an empty input
- **WHEN** an IR `Aggregate(group = [?c], count(*) as ?n)` is executed over an empty input
- **THEN** the result has zero rows

### Requirement: Parameters
The IR SHALL allow named parameters wherever a constant is allowed: in pattern positions, in expressions, in Values cells, and as skip and limit. Parameters SHALL be resolved from the parameter map supplied at execution, and SHALL then be treated exactly like constants, including dictionary lookup and short-circuiting. A parameter that is referenced but not supplied SHALL fail the query with an `InvalidQuery` error that names it, before any SQL is executed.

#### Scenario: Parameter as a pattern constant
- **WHEN** an IR `TriplePattern(?p, v:email, $email)` is executed with `email = "a@example.org"`
- **THEN** the result is the same as with the constant `"a@example.org"` in that position

#### Scenario: Missing parameter
- **WHEN** an IR references `$email` and the parameter map has no `email`
- **THEN** execution fails with `InvalidQuery` naming `email`

#### Scenario: Unknown parameter value short-circuits
- **WHEN** a parameter's value is an IRI that is missing from the dictionary
- **THEN** its pattern is empty, as it would be for a constant

### Requirement: Structural validation
The executor SHALL reject a structurally invalid IR with `InvalidQuery`, before executing anything. Invalid IR includes: an Extend that binds a variable its input already binds; an Aggregate output variable that collides with a grouping variable; a Values row whose width differs from its variable list; a negative skip or limit; and a virtual-predicate pattern that binds an eid variable. A PathPattern whose two endpoints are both unbound after planning SHALL be rejected with `Unsupported`.

#### Scenario: Extend rebinds a bound variable
- **WHEN** an IR `Extend(?a := 1, TriplePattern(?a, v:p, ?b))` is executed
- **THEN** execution fails with `InvalidQuery`

#### Scenario: Ragged Values
- **WHEN** a Values operator declares variables `[?x, ?y]` and one row has three cells
- **THEN** execution fails with `InvalidQuery`

#### Scenario: Path with no bound endpoint
- **WHEN** an IR contains a PathPattern whose start and end are both variables that no other operator binds
- **THEN** execution fails with `Unsupported`
