# Local MCP memory adapter

## Status

Implemented. Crate and binary `tiramemsu-mcp` (lib `tiramemsu_mcp`: `Server`, `Config`, `Budget`, `parse_args`, `ToolError`); tools `assert`, `confirm`, `supersede`, `query`, `dependents`, `export_bundle`, `import_bundle`, `text_search`; facade additions `SparqlOptions::query_only`, `Solutions::provenance_gaps` / `provenance_complete()` and `ProvenanceGap`; bridge `queryOnly` and `provenanceGaps`; `tiramemsu-json` made publishable.

## Why

The embedded memory verbs need a small agent-tool interface. A local adapter can expose the existing model without introducing a database server.

## What Changes

Add a separate opt-in local stdio MCP package with typed memory verbs, read-only mode, bounded queries, and explicit provenance metadata.

## Capabilities

### New Capabilities

- `mcp-memory-adapter`: local mcp memory adapter.

### Modified Capabilities

None in this draft. Implementation must add delta modifications if an existing public requirement must change.

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No HTTP transport, multi-user authorization, or remote server.
- No automatic source extraction, embeddings, or tool-driven code execution.
