## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Keep protocol dependencies outside tm-core and outside default facade dependencies. A server configuration selects an explicit database path and write policy. Tools initially cover assert, confirm, supersede, query, dependents, export_bundle, and import_bundle. Use the existing JSON term forms and error codes. Tool requests cannot replace the configured path or execute raw SQL. Apply query-budgets at the adapter boundary. Free text remains data and never changes tool authorization. Support graph queries immediately; retrieval can be added after add-text-retrieval.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

- **Package:** a workspace crate `crates/tiramemsu-mcp` with a library (`Server` handles one JSON-RPC line, `Server::serve` drives stdio) and a binary `tiramemsu-mcp`. It depends on `tiramemsu`, `tiramemsu-json` and `serde_json` only; the protocol layer (JSON-RPC 2.0, newline-delimited, `initialize`, `ping`, `tools/list`, `tools/call`, notifications, batches) is hand-rolled, so neither `tm-core` nor the facade gains a dependency. `tiramemsu-json` drops `publish = false` so the adapter can be published after it.
- **Protocol revisions:** `2025-06-18` is served; `2025-03-26` and `2024-11-05` are accepted and echoed; anything else is answered with `2025-06-18`. `structuredContent` is sent from `2025-06-18` on, text content always.
- **Configuration:** command-line only: `--db` (required), `--read-only`, `--text-index`, `--timeout-ms` (default 30000), `--reader-timeout-ms`, `--max-rows` (10000), `--max-bytes` (8 MiB); `0` is unbounded. Every tool call runs under that budget through the bridge's `budget` object.
- **Write policy:** read-only mode hides write tools from `tools/list` and refuses them with `ReadOnly` before argument parsing. `query` is read-only in every mode: SPARQL runs with the new `SparqlOptions::query_only` (an update is `Unsupported` before anything runs) and Cypher on a view. No raw SQL.
- **Path policy:** each tool declares its argument keys (`additionalProperties: false`); a path-like key (`path`, `db`, `database`, `file`, `uri`, ...) is `PathNotAllowed`, any other undeclared key `InvalidArgument`.
- **Provenance coverage:** `tm-sparql`'s provenance pass now records `ProvenanceGap::RecursivePath` for `Op::Path` regions in `ProvenancePlan::gaps`, surfaced as `Solutions::provenance_gaps` and `provenance_complete()`, and as `provenanceGaps` on the bridge. A text match (`Op::Text`) now cites its matched statement instead of adding nothing. The `query` tool defaults provenance on for a SPARQL `SELECT` (detected from the first keyword after the prologue) and reports `complete`, `incomplete` (with gaps) or `unavailable` (Cypher, `ASK`, `CONSTRUCT`, or off); an explicit unsupported combination is an error, not a retry.
- **Errors:** tool failures are results with `isError` and `{code, message}` using the bridge codes plus `ReadOnly` and `PathNotAllowed`; a malformed bundle is pre-validated with `Bundle::from_json` and reported as `InvalidArgument` before the transaction. Protocol faults are JSON-RPC errors, and an unknown tool is `-32602`.
- **Deferred:** `retract`, `history` and `schema` tools from the older contract in `lat.md/api.md` (SPARQL on a history view covers history), HTTP transport, per-call budget overrides and cancellation over `notifications/cancelled` (calls run synchronously on one thread).
