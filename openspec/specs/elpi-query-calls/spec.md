# elpi-query-calls Specification

## Purpose
TBD - created by archiving change add-elpi-query-calls. Update Purpose after archive.

## Requirements

### Requirement: ELPI goal calls
A SPARQL `SERVICE <urn:tiramemsu:elpi:query>` block and a Cypher `CALL tiramemsu.elpi.query(program, goal)` SHALL solve the goal against the program in the query's view: a stored program (`elpi:program <iri>`, or an IRI string in Cypher) or program text (`elpi:text "…"`, or `{text: "…"}`). Each goal variable SHALL bind the query variable of the same name, one row per solution, with the same rows as `View::elpi`. The call SHALL run once per query execution. `elpi:validFrom` and `elpi:validTo` SHALL stay unbound, because a goal reads one valid instant. A goal value without a dictionary id SHALL fail the query with `Unsupported`.

#### Scenario: Join with a pattern
- **WHEN** a SPARQL query joins `SERVICE <urn:tiramemsu:elpi:query>`, with the goal `reports (iri "…alice") Y` and variable `?Y`, to `?Y v:email ?mail`
- **THEN** it returns the emails of everyone alice reports to

#### Scenario: Cypher call then match
- **WHEN** `CALL tiramemsu.elpi.query('v:reporting', 'reports (iri "…alice") Y') YIELD Y` is followed by a `MATCH` on `Y`
- **THEN** it returns the same people as the SPARQL query

#### Scenario: Computed value refused
- **WHEN** the goal binds a string computed by the program that is not stored
- **THEN** the query fails with `Unsupported`

### Requirement: ELPI call feature gating
Without the facade feature `elpi`, an ELPI goal call SHALL parse, and SHALL fail at execution with `Unsupported`.

#### Scenario: No ELPI provider
- **WHEN** the facade is built with `datalog` but without `elpi` and a query uses `SERVICE <urn:tiramemsu:elpi:query>`
- **THEN** the query fails with `Unsupported`

### Requirement: ELPI bridge operations
The JSON bridge SHALL offer the read operations `elpi`, `elpiCheck`, `elpiConclude` and `elpiProgramInfo`, the transaction ops `storeElpiProgram` and `enforceElpi`, and `materializeElpi`, behind its feature `elpi`. Without the feature they SHALL fail with `Unsupported`. The MCP server, built with `elpi`, SHALL offer the tools `elpi_query` and `elpi_check`, and the write tools `store_elpi_program`, `enforce_elpi` and `materialize_elpi`; read-only mode SHALL hide the write tools.

#### Scenario: Bridge round trip
- **WHEN** a program is stored through `storeElpiProgram` and run through `elpi`, `elpiConclude`, `elpiCheck` and `materializeElpi`
- **THEN** each returns the result of its Rust call in the bridge's JSON shapes

#### Scenario: Bridge without the feature
- **WHEN** the bridge is built without `elpi`
- **THEN** every ELPI operation fails with `Unsupported`

#### Scenario: MCP tools
- **WHEN** an agent stores a program, queries it, materializes it, checks it and enforces it through the MCP tools
- **THEN** each tool succeeds, and in read-only mode only `elpi_query` and `elpi_check` are listed
