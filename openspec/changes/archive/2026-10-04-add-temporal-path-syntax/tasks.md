## Status

Implemented. SPARQL: `SERVICE <urn:tiramemsu:tm:timeRespecting[/<ms | xsd:date | xsd:dateTime | $name>]>` with `?end tm:arrival ?t`, and `SparqlOptions::params`. Cypher: `MATCH TIME RESPECTING [AFTER t] [ARRIVAL AS name]`. IR: `tm_ir::TemporalPath` on `PathPattern.time_respecting`, `PathPattern.hop_cap`. Completeness: `tm_ir::PathCompleteness` (`Exhaustive`, `StoppedAtBound`, `StoppedAtCap`), `View::path_report` / `PathReport`, `PathArgs::capped`, `Solutions::path_completeness`, `CypherResult::path_completeness`, `tm_exec::path::report`, the `hopCap` part of the `tm_path` view text. JSON bridge: `sparql` `params` and `pathCompleteness`, `cypher` `pathCompleteness`, `path` `capped` and `completeness`. Node: `sparql(text, { params, pathCompleteness })`, `cypher(text, params, { pathCompleteness })`, `View.pathReport`, `PathOptions.capped`. Python: `sparql(params=, path_completeness=)`, `cypher(path_completeness=)`, `View.path_report`, `path(capped=)`, `PathCompleteness`, `PathReport`.

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement temporal path syntax and completeness reporting behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec, and archive only after implementation is complete.

> Notes: acceptance tests are `crates/tiramemsu/tests/temporal_path_syntax.rs` (every scenario of the delta spec, in both languages against the Rust API), `crates/tm-sparql/tests/lower_temporal_paths.rs`, the two `time_respecting_match_*` tests of `crates/tm-cypher/tests/golden_ir.rs`, the bridge test `temporal_path_syntax_and_completeness_cross_the_bridge`, the `report.rs` unit test, and one Node and one Python test. No storage format change: the feature is query-side only. The change is not archived.
