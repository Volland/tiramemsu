## MODIFIED Requirements

### Requirement: Cost-based join order
The native planner SHALL order joins from estimated cardinalities derived from exact predicate statistics and bounded range probes. It SHALL consider every pattern as the first access path, complete each order greedily with the connected pattern of fewest estimated matches per row, and choose the order with the fewest estimated intermediate rows in total.

#### Scenario: Rare predicate first
- **WHEN** a query joins a pattern on a predicate with 10 statements to one on a predicate with 1 000 000 statements
- **THEN** explain shows the rare pattern as the first access path

#### Scenario: Selective constant before a small pattern
- **WHEN** a chain query has a pattern with a constant object that matches few subjects at one end and a pattern on a small predicate at the other end, and starting from the small pattern would grow the intermediate result through the chain
- **THEN** explain shows the pattern with the constant as the first access path

## ADDED Requirements

### Requirement: Constant equalities seek
When a conjunct of a basic graph pattern's filters compares one of its variables for equality with a constant that the native evaluator compares by identity (a constant that is in the dictionary and is neither a number nor a date-time), the native runner SHALL bind the variable to the constant's ObjectId before the join, so that the patterns using it seek on it. The conjunct SHALL still be checked, and the answers SHALL equal those of the SQL runner.

#### Scenario: String equality becomes a seek
- **WHEN** `SELECT ?x WHERE { ?x v:name ?n FILTER(?n = "n7") }` runs on redb
- **THEN** explain shows `?n` bound to the constant and an index nested loop over one input row, and the rows equal those on SQLite

#### Scenario: Numeric equality is not pinned
- **WHEN** `SELECT ?x WHERE { ?x v:age ?a FILTER(?a = 7) }` runs on redb over ages stored as the integer `7` and the double `7.0`
- **THEN** explain shows no binding of `?a`, and both statements are returned, as on SQLite
