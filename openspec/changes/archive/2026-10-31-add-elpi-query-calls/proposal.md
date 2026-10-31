# ELPI goals from SPARQL, Cypher, the bridge and MCP

Source: decision E11 of `lat.md/elpi.md` (a proposed default, 2026-10-31). Change 4 of four; depends on `add-elpi-programs`.

## Why

A SPARQL or Cypher query that calls an ELPI goal joins ELPI answers with ordinary patterns in one query, the way Datalog goals already can (DL17). Agents and wrappers reach the engine through the JSON bridge and the MCP server, not through Rust.

## What Changes

- **SPARQL:** `SERVICE <urn:tiramemsu:elpi:query> { [] elpi:program <p> | elpi:text "…" ; elpi:goal "…" }`, with `elpi:` predeclared.
- **Cypher:** `CALL tiramemsu.elpi.query(program, goal) YIELD …`.
- **One call path for both languages.** The IR leaf `DatalogPattern` and the plan node `PDatalog` gain a `language` (`GoalLanguage::Datalog | Elpi`). The `tm_datalog` table function and the native operator pick the provider for that language from `GoalProviders`, and `QueryEngine::with_elpi_provider` registers ELPI's. The facade registers an ELPI provider over `tm-elpi` when built with `exec` and `elpi`. ELPI rows have no interval, so `validFrom` and `validTo` stay unbound.
- **JSON bridge (feature `elpi`):** `elpi`, `elpiCheck`, `elpiConclude`, `elpiProgramInfo`, `storeElpiProgram`, `enforceElpi` and `materializeElpi`.
- **MCP (feature `elpi`):** `elpi_query`, `elpi_check`, `store_elpi_program`, `enforce_elpi` and `materialize_elpi`.

## Capabilities

### New Capabilities

- `elpi-query-calls`.

## Impact

- `tm-ir` (`GoalLanguage`), `tm-exec` (`GoalProviders`, the table function, the engine), `tm-native` (the operator), `tm-sparql` and `tm-cypher` (lowerings), the facade (`src/elpi_call.rs`), `bindings/json` (`src/elpi.rs`), `tiramemsu-mcp`.
- `tm-core` vocabulary: `ELPI`, `ELPI_QUERY`, `ELPI_GOAL`, `ELPI_PROGRAM`, `ELPI_TEXT`, `ELPI_VALID_FROM`, `ELPI_VALID_TO`.
