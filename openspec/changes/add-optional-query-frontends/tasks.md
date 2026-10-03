## Status

Implemented. Final contract: `tiramemsu` cargo features `exec` (`tm-ir`, `tm-exec`: `execute_ir`, `explain_ir`, `View::path*`, `PathArgs`, LFTJ, `ir`, the engine `OpenOptions` fields), `sparql` (implies `exec`; `tm-sparql`: `View::sparql*`, `SparqlResult`, `Solutions`) and `cypher` (implies `exec`; `tm-cypher`: `View::cypher`, `Db::cypher_write*`, `TxCypher`); `default = ["sparql", "cypher"]`, which also gates saved answers. `default-features = false` is the core tier. RDF terms and the N-Triples writer moved to `tm_core::rdf` (re-exported by `tm-sparql`), so bundle formats need no parser. The JSON bridge and `tiramemsu-mcp` enable `sparql` and `cypher` explicitly. `scripts/feature-matrix.sh` and the CI `features` job check the combinations.

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement optional query frontend dependencies behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec (archiving is left for a separate step).
