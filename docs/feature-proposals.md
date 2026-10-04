# Proposed features after the architecture review

All changes below are **proposed and unimplemented**. Implementation tasks remain unchecked. The current patch only repairs resource cleanup and benchmark errors; it does not activate these capabilities.

| Change | Scope |
|---|---|
| [add-query-budgets](../openspec/changes/add-query-budgets/proposal.md) | Query budgets and bounded reader acquisition |
| [add-bulk-import](../openspec/changes/add-bulk-import/proposal.md) | Bulk import with deferred statistics |
| [add-text-retrieval](../openspec/changes/add-text-retrieval/proposal.md) | Text recall with evidence ranking |
| [add-mcp-adapter](../openspec/changes/add-mcp-adapter/proposal.md) | Local MCP memory adapter |
| [add-saved-answer-invalidation](../openspec/changes/add-saved-answer-invalidation/proposal.md) | Saved answers and conservative invalidation |
| [add-temporal-path-syntax](../openspec/changes/add-temporal-path-syntax/proposal.md) | Temporal path syntax and completeness reporting |
| [add-lftj-operator](../openspec/changes/add-lftj-operator/proposal.md) | Native cyclic-join operator |
| [add-memory-conflict-review](../openspec/changes/add-memory-conflict-review/proposal.md) | Conflict inspection and import previews |
| [add-optional-query-frontends](../openspec/changes/add-optional-query-frontends/proposal.md) | Optional query frontend dependencies |
| [add-wasm-sqlite-host](../openspec/changes/add-wasm-sqlite-host/proposal.md) | WASM SQLite host |

## Suggested order

Query budgets and bulk import can follow the cleanup patch. Text recall and the MCP adapter are the next agent-facing work; the adapter uses query budgets. Saved-answer invalidation depends on explicit provenance coverage and conservative insert invalidation. Temporal syntax and cyclic joins fit the existing query architecture. Conflict previews use existing bundles and dry runs. Optional frontends should precede WASM packaging where possible.

History lookup tuning remains a separate investigation: the review measured a slowdown but did not isolate a schema or planner defect. No history-index rewrite is proposed as a fast patch.

Crypto-shredding and replica-origin allocation remain existing roadmap items; these drafts do not implement either.

## Patch verification

The preceding cleanup patch was verified with 1,131 passing workspace tests and zero failures, including four new regression tests that failed before the fix. Clippy with warnings denied, formatting, both corrected benchmark smoke runs, strict OpenSpec validation (48 items), and `lat check` passed. The original panic probe now completes subsequent writes and reads successfully. Historical review logs remain available alongside `patch-*.log` evidence in `docs/review-results/`.
