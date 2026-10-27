# native-query-execution Specification

## Purpose
The native query runner (`tm-native`) executes the whole query IR over the storage traits on redb, with the same answers as the SQL runner on SQLite, cost-based hybrid joins, worst-case-optimal cyclic joins, late decoding, parallel join steps, budgets and explain.

## Requirements

### Requirement: Same answers as the SQL runner
For every query that `tm-exec` answers on a SQLite database, the native runner SHALL return the same solutions with the same multiplicities, the same order where the query fixes one, and the same error kind, on a redb database with the same history. This SHALL hold for SPARQL and Cypher, every time scope and view, graph selections, paths, recall patterns, provenance and writes from query languages.

#### Scenario: Differential corpus on redb
- **WHEN** the dialect differential corpus, the SPARQL and Cypher suites, the recipes and the path and recall suites run on a redb database
- **THEN** every result equals the result on SQLite

#### Scenario: Random queries agree
- **WHEN** the property test generates random basic graph patterns with filters, optionals, unions and aggregates over random histories
- **THEN** the native runner and `tm-exec` return the same multiset of solutions

### Requirement: Cost-based join order
The native planner SHALL order joins from estimated cardinalities derived from exact predicate statistics and bounded range probes. It SHALL consider every pattern as the first access path, complete each order greedily with the connected pattern of fewest estimated matches per row, and choose the order with the fewest estimated intermediate rows in total.

#### Scenario: Rare predicate first
- **WHEN** a query joins a pattern on a predicate with 10 statements to one on a predicate with 1 000 000 statements
- **THEN** explain shows the rare pattern as the first access path

#### Scenario: Selective constant before a small pattern
- **WHEN** a chain query has a pattern with a constant object that matches few subjects at one end and a pattern on a small predicate at the other end, and starting from the small pattern would grow the intermediate result through the chain
- **THEN** explain shows the pattern with the constant as the first access path

### Requirement: Worst-case-optimal cyclic joins
For a cyclic core of a basic graph pattern the native runner SHALL use a multiway intersection join, so that its work is bounded by the worst-case output size of the core rather than by an intermediate binary join result.

#### Scenario: Skewed triangle
- **WHEN** a triangle query runs on the hub-and-spoke fixture of the triangle benchmark
- **THEN** explain shows a generic-join step for the triangle and the number of keys read is within a constant factor of the AGM bound of the fixture

#### Scenario: Acyclic stays binary
- **WHEN** a star or chain query runs
- **THEN** explain shows no generic-join step

### Requirement: Late decoding
The native runner SHALL evaluate joins, id equality, graph membership and time filters on ObjectIds and SHALL decode dictionary terms only for projected columns and for expressions that need values.

#### Scenario: Count does not decode
- **WHEN** `SELECT (COUNT(*) AS ?n) WHERE { ?a :knows ?b . ?b :knows ?c }` runs
- **THEN** no dictionary term is read

### Requirement: Parallel scans
Access paths and hash builds estimated above the parallel threshold SHALL run in parallel morsels; `parallel_threshold = 0` SHALL disable parallelism; results SHALL be identical either way.

#### Scenario: Same result with and without parallelism
- **WHEN** a large scan query runs with the default threshold and with parallelism disabled
- **THEN** both return the same solutions

### Requirement: Budgets on the native runner
The native runner SHALL honour `QueryBudget` deadlines, cancellation tokens, row limits and byte limits, checking them at least once per batch, and SHALL return the same typed errors as `tm-exec`.

#### Scenario: Cancel a long native join
- **WHEN** a cancellation token fires while a large cyclic join runs
- **THEN** the query fails with `Cancelled` and no partial result is returned

### Requirement: Native explain
`explain` on a redb database SHALL describe the native plan: the access path and permutation of each pattern, the join kind of each step, and estimated rows.

#### Scenario: Explain names the permutation
- **WHEN** `explain_sparql` runs on `SELECT ?s WHERE { ?s :p :o }`
- **THEN** the plan names a `pos` live scan with the estimate from the statistics

### Requirement: Constant equalities seek
When a conjunct of a basic graph pattern's filters compares one of its variables for equality with a constant that the native evaluator compares by identity (a constant that is in the dictionary and is neither a number nor a date-time), the native runner SHALL bind the variable to the constant's ObjectId before the join, so that the patterns using it seek on it. The conjunct SHALL still be checked, and the answers SHALL equal those of the SQL runner.

#### Scenario: String equality becomes a seek
- **WHEN** `SELECT ?x WHERE { ?x v:name ?n FILTER(?n = "n7") }` runs on redb
- **THEN** explain shows `?n` bound to the constant and an index nested loop over one input row, and the rows equal those on SQLite

#### Scenario: Numeric equality is not pinned
- **WHEN** `SELECT ?x WHERE { ?x v:age ?a FILTER(?a = 7) }` runs on redb over ages stored as the integer `7` and the double `7.0`
- **THEN** explain shows no binding of `?a`, and both statements are returned, as on SQLite
