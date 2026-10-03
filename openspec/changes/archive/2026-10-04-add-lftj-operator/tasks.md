## Status

Implemented. Final names: `LftjConfig { enabled, min_rows_estimate }` in `OpenOptions::planner`, `LftjOperator` / `tm_lftj(spec)`, `RouteNote::{LftjUnsupportedShape, LftjBelowEstimate, LftjNative}` beside the existing `CyclicLftjDisabled` and `LftjUnavailable`, `View::explain_sparql`, JSON `lftj` / `lftjMinRows` / `explainSparql`, Node `explainSparql`, Python `explain_sparql`. Archiving is left to a later step.

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement native cyclic-join operator behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec, and archive only after implementation is complete (documentation and validation done; the change is not archived yet).
