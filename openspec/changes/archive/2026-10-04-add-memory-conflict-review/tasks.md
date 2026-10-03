## Status

Implemented. Final API: `View::conflicts(&ConflictQuery) -> Vec<Conflict>` (`Conflict`, `ConflictValue`, `ConflictEvidence`), `Db::preview_bundle(&Bundle) -> BundlePreview` (`PreviewScope`, `IdUsage`, `Tx::id_usage`); JSON bridge `conflicts` and `previewBundle`; Node `View.conflicts` / `Database.previewBundle`; Python `View.conflicts` / `Database.preview_bundle`; MCP read tools `conflicts` and `preview_bundle`. Applying stays `Tx::import_bundle` (`importBundle`, MCP `import_bundle`).

## Implementation

- [x] 1. Finalize the public contract, capability boundaries, dependency requirements, and compatibility/migration plan.
- [x] 2. Add failing acceptance tests for every proposed scenario, including errors and existing temporal invariants.
- [x] 3. Implement conflict inspection and import previews behind the planned opt-in boundary.
- [x] 4. Run the feature-specific tests, relevant differential suites, and documented performance or dependency checks.
- [x] 5. Update Rust/binding documentation and lat.md, validate OpenSpec (archiving is left for a separate step).
