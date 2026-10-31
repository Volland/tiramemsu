## 1. Calls

- [x] 1.1 `GoalLanguage` on the IR leaf and the plan node; `GoalProviders` and `QueryEngine::with_elpi_provider` in `tm-exec`; the table function and the native operator pick the provider by language. (Design refined: one call path with a language tag, not a second `ElpiProvider` trait.)
- [x] 1.2 SPARQL `SERVICE <urn:tiramemsu:elpi:query>` and Cypher `CALL tiramemsu.elpi.query` lowerings; the `elpi:` prefix is predeclared.
- [x] 1.3 The facade's provider over `tm-elpi` and `tm-elpi-store` (`src/elpi_call.rs`).

## 2. Surfaces

- [x] 2.1 JSON bridge ops behind the feature `elpi`, and `Unsupported` without it.
- [x] 2.2 MCP tools behind the feature `elpi`; read-only mode hides the write tools.

## 3. Tests

- [x] 3.1 Scenario tests with `@tg: verifies::` (`crates/tiramemsu/tests/elpi_calls.rs`, `bindings/json/tests/elpi.rs`, `crates/tiramemsu-mcp/tests/server.rs`).
