## Status

Implemented. Crate and binary `tiramemsu-mcp` (lib `tiramemsu_mcp`: `Server`, `Config`, `Budget`, `parse_args`, `ToolError`); tools `assert`, `confirm`, `supersede`, `query`, `dependents`, `export_bundle`, `import_bundle`, `text_search`; facade additions `SparqlOptions::query_only`, `Solutions::provenance_gaps` / `provenance_complete()` and `ProvenanceGap`; bridge `queryOnly` and `provenanceGaps`; `tiramemsu-json` made publishable.

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement local mcp memory adapter behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec, and archive only after implementation is complete.
