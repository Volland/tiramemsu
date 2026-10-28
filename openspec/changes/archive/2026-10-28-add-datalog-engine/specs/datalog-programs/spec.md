## Purpose

Datalog programs stored as memory: how a program is kept, versioned and inspected, and the read operations every surface offers.

## ADDED Requirements

### Requirement: Store a program
`TxDatalog::store_program(program, text)` SHALL parse and analyse the text and keep it as one statement `(program dl:source "text")`, where `dl:` is `urn:tiramemsu:dl:`; storing a different text SHALL supersede the previous statement, and storing the same text SHALL change nothing and return the existing eid. A text that fails parsing or analysis SHALL fail the transaction with that error. `Program::Stored(program)` SHALL run the text live in the view; an unknown program SHALL fail with `UnknownProgram`.

#### Scenario: Store and run
- **WHEN** the `reports` program is stored as `ex:reporting` and `?- reports(ex:alice, X).` runs with `Program::Stored(ex:reporting)`
- **THEN** it returns the same rows as running the text

#### Scenario: Same text is idempotent
- **WHEN** the same text is stored twice
- **THEN** the second call returns the first eid and writes nothing

#### Scenario: Invalid text is refused
- **WHEN** an unstratifiable text is stored
- **THEN** the transaction fails with `DatalogUnstratifiable` and nothing is written

### Requirement: Programs have transaction time
A stored program SHALL be read in the view's transaction time, so a run on `as_of(t)` uses the text believed at `t`.

#### Scenario: As-of uses the old program
- **WHEN** `ex:reporting` is stored in transaction 2 as the direct-manager rule only and superseded in transaction 5 by the transitive program
- **THEN** a run on `as_of(Tx(3))` returns only direct managers

### Requirement: Program information
`View::program_info(program)` SHALL return the program node, its statement, its text, its rules, goals and constraints, its strata with monotonicity, the stored predicates it reads, whether it uses a variable predicate, and whether it recurses through a metric operator or through value invention.

#### Scenario: Info of a stored program
- **WHEN** `program_info(ex:reporting)` is called
- **THEN** it reports two rules, one recursive monotonic stratum and the predicate `ex:manager`

### Requirement: Rule language conflict
A node SHALL hold at most one live rule text: storing a Datalog program on a node with a live `n3:source`, or N3 rules on a node with a live `dl:source`, SHALL fail with `RuleLanguageConflict`.

#### Scenario: N3 node refused
- **WHEN** `ex:r` holds N3 rules and `store_program(ex:r, …)` is called
- **THEN** it fails with `RuleLanguageConflict`

### Requirement: Datalog read surfaces
The JSON bridge with its feature `datalog` SHALL offer the read ops `datalog`, `datalogReason`, `programInfo`, `explainDatalog` and `check` and the transaction op `storeProgram`, with N3's `view`, `budget`, `scope`, `window`, `limits` and `requireComplete` shapes; without the feature they SHALL fail with `Unsupported`. Node, Python and wasm SHALL expose them through the bridge. The MCP server SHALL offer `datalog_query` and `check_constraints` as read tools and `store_program` as a write tool hidden in read-only mode. The CLI SHALL offer `tm datalog <file> <program> [goal]`, `tm check <file> <program>` and `tm program put|show`.

#### Scenario: Bridge goal query
- **WHEN** the bridge receives `{"op": "datalog", "program": "<text>", "goal": "reports(ex:alice, X)"}`
- **THEN** it returns `{"rows": [...], "status": "complete", ...}` with bridge terms and intervals

#### Scenario: Bridge without the feature
- **WHEN** the bridge is built without `datalog` and receives the op `datalog`
- **THEN** it fails with code `Unsupported`

#### Scenario: Read-only MCP hides writes
- **WHEN** the MCP server runs in read-only mode
- **THEN** `datalog_query` and `check_constraints` are listed and `store_program` is not

#### Scenario: CLI goal query
- **WHEN** `tm datalog mem.db reporting.dl "reports(ex:alice, X)"` runs
- **THEN** it prints the rows with their intervals and exits with status 0
