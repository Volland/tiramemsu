## Status

Implemented. Final names: `View::text_search`, `TextQuery`/`TextMode`/`TextHit`/`TextEvidence`, `OpenOptions::text_index`, `Db::enable_text_index`, `Db::rebuild_text_index`, `Error::TextIndexUnavailable`, storage format 2, SPARQL `tm:textMatch`, Cypher `tiramemsu.text.search`, bridge `textSearch`.

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement text recall with evidence ranking behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec, and archive only after implementation is complete (archiving is left to a separate step).
