# temporal-language-paths Specification

## Purpose
TBD - created by archiving change add-temporal-path-syntax. Update Purpose after archive.

## Requirements

### Requirement: Shared causal journey semantics
Both query languages SHALL lower temporal path modifiers to the existing time-respecting native semantics and SHALL use the selected transaction and graph view. In SPARQL the modifier is the scope `SERVICE <urn:tiramemsu:tm:timeRespecting[/<t>]> { … }`; in Cypher it is `MATCH TIME RESPECTING [AFTER t] [ARRIVAL AS name]`. Both are opt-in Tiramemsu extensions, and a time-respecting path SHALL need a bound start.

#### Scenario: Reverse valid-time order
- **WHEN** two consecutive edges cannot be taken in nondecreasing valid time
- **THEN** the temporal path does not match even if an ordinary path does

#### Scenario: Virtual layer hop
- **WHEN** a temporal path crosses sys:subject
- **THEN** the virtual hop preserves the current arrival instant

#### Scenario: View and graph scope
- **WHEN** a temporal path is read under an as-of scope or inside a `GRAPH` block
- **THEN** it sees the facts of that snapshot and only member statements, like the Rust API on the same view

#### Scenario: Start not bound
- **WHEN** a temporal path has only its end bound
- **THEN** the query fails with `Unsupported` instead of running the journey backwards

### Requirement: Earliest arrival binding
A temporal reachability query SHALL be able to bind the earliest arrival per endpoint, with explicit handling of an unbounded initial instant: SPARQL with `?end tm:arrival ?t`, Cypher with `ARRIVAL AS name`. An arrival of −∞ SHALL leave the SPARQL variable unbound and bind Cypher `null`. Without an arrival binding the rows SHALL keep the columns of the same query without the modifier.

#### Scenario: Multiple journeys
- **WHEN** two valid journeys reach an endpoint at different times
- **THEN** reachability binds the earlier arrival

#### Scenario: Parameterized start
- **WHEN** the start instant is supplied as an epoch-millisecond parameter
- **THEN** both languages produce the same endpoints and arrivals as the Rust API

#### Scenario: Unbounded initial instant
- **WHEN** no start is given and no traversed statement has a valid-from instant
- **THEN** the arrival is unbound in SPARQL and `null` in Cypher

#### Scenario: Malformed modifier
- **WHEN** the modifier or the arrival binding is malformed (an invalid start, `tm:arrival` outside a scope, a Cypher modifier without a variable-length relationship)
- **THEN** the query fails with a `Parse` error before anything runs

### Requirement: Explicit search completeness
Path results SHALL distinguish exhaustive evaluation within an explicit query bound from evaluation stopped by a configured hop cap, as `Exhaustive`, `StoppedAtBound` or `StoppedAtCap`. State exhaustion SHALL remain a typed error. The metadata SHALL be available from the Rust API, the JSON bridge and the bindings without changing ordinary responses unless it is requested.

#### Scenario: Configured cap
- **WHEN** an unbounded trail reaches the configured hop cap
- **THEN** metadata identifies the cap and does not claim unbounded exhaustive evaluation

#### Scenario: Explicit bound
- **WHEN** an explicit hop bound stops a search that could continue
- **THEN** metadata reports the bound, and a search that runs out of states is reported as exhaustive

#### Scenario: State guard
- **WHEN** the search exceeds its state limit
- **THEN** the query fails with PathLimitExceeded rather than returning a successful prefix
