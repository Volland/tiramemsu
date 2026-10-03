## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Keep default builds equivalent to today. Treat the existing sparql feature name as a compatibility concern: document migration explicitly and avoid silently dropping default Cypher support. Separate core, shared execution/paths, SPARQL, and Cypher feature sets. Audit bundle formatting and JSON bridge dependencies, which currently rely on SPARQL serialization. Core-only bundle JSON and transactions must not pull a parser transitively. Query-engine options and capability errors remain runtime concerns only when execution is compiled in. Validate dependency trees and a build/test matrix, not just cfg compilation.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

- **Features.** `tiramemsu` declares `tm-ir`, `tm-exec`, `tm-sparql` and `tm-cypher` as optional dependencies behind `exec = ["dep:tm-ir", "dep:tm-exec"]`, `sparql = ["exec", "dep:tm-sparql"]` and `cypher = ["exec", "dep:tm-cypher"]`, with `default = ["sparql", "cypher"]`. The default build is today's facade. `default-features = false` is the core tier: `Db`, transactions, speculation, views and `triples`, dependents, bundles and their formats, bundle previews, conflict review, text recall (`View::text_search`), bulk import and budgets.
- **Gating.** `exec` gates `View::execute_ir`, `explain_ir`, `path`, `path_with`, `path_report`, `PathArgs`, `PathReport`, the `ir` module, the `tm-exec` re-exports (LFTJ, `PlannerOptions`, `QueryResult`, `Explain`, ...), `Db::path_max_hops` and the `OpenOptions` fields `planner`, `query_engine`, `path_max_hops`, `path_max_states` (and the hidden `native_operators`). Without `exec` the facade's engine type is uninhabited, so no engine is ever installed (the old `query_engine: false` behaviour, at compile time). `sparql` gates `View::sparql`, `sparql_with`, `explain_sparql`, `SparqlOptions`, `SparqlResult`, `Solutions`, `ProvenanceGap` and `sparql_frontend`; `cypher` gates `View::cypher`, `Db::cypher_write`, `cypher_write_budgeted`, `TxCypher`, `CypherParams`/`CypherValue`/`CypherResult` and `cypher_frontend`. Saved answers persist queries of both languages (`SavedQuery.params` is `CypherParams`, `SavedAnswer::solutions` returns `Solutions`), so they need both front ends. Text recall stays in the core; its SPARQL (`tm:textMatch`) and Cypher (`tiramemsu.text.search`) entry points come with their front ends. Temporal path syntax lives in the front ends; `PathCompleteness` comes with `exec`.
- **Serialisation without a parser.** `RdfTerm`, `RdfTriple`, `render`, `datetime_lexical` and the N-Triples writer moved from `tm-sparql::results::{term, nt}` to `tm_core::rdf` (`write_ntriples`); `tm-sparql` re-exports them under the old paths, and the facade re-exports `RdfTerm`/`RdfTriple` in every build. `BundleFormat` (JSON and N-Triples) uses them, so a core-only bundle export and import never link a parser. Settings (`@vocab`, prefixes) are read through `tm_core::mapping` by both front ends.
- **Compatibility.** The old `sparql = []` feature only switched the SPARQL side of the differential suite; `default-features = false` still built both front ends. Now `sparql` selects the front end and the differential suite compiles when both front ends do. The facade README's "Cargo features" section documents the migration: dependants with `default-features = false` add `features = ["sparql", "cypher"]` (or a subset). The JSON bridge and `tiramemsu-mcp` declare `features = ["sparql", "cypher"]`; the Node and Python bindings inherit them through the bridge.
- **Docs.** The README (a doctest tour that uses both languages) is the crate doc only when both front ends are enabled; reduced builds get a short feature summary. Doctests on core items use the core API, and intra-doc links from core items to gated items are plain code spans, so `cargo doc` with `-D warnings` passes in every combination.
- **Tests and CI.** Test files are gated by the features they use (`#![cfg(...)]`), with per-test gates where a core test only verified through SPARQL. `tests/feature_matrix.rs` holds runtime smoke tests: core operations, bundle formats, the recent core features and a cross-build file handoff run in every combination; shared execution, SPARQL, Cypher and the default surface run where compiled. `scripts/feature-matrix.sh <core|exec|sparql|cypher|default|bindings> [deps|check|test|handoff]` asserts `cargo tree -e normal` (present and absent crates, including `spargebra`, `peg` and `open-cypher`), checks, tests and hands a file to and from the default build; the CI `features` job runs it per combination with clippy, the default examples and a core-only `cargo doc`. No storage format change.
