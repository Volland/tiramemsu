# Conflict inspection and import previews

## Status

Implemented. Final API: `View::conflicts(&ConflictQuery) -> Vec<Conflict>` (`Conflict`, `ConflictValue`, `ConflictEvidence`), `Db::preview_bundle(&Bundle) -> BundlePreview` (`PreviewScope`, `IdUsage`, `Tx::id_usage`); JSON bridge `conflicts` and `previewBundle`; Node `View.conflicts` / `Database.previewBundle`; Python `View.conflicts` / `Database.preview_bundle`; MCP read tools `conflicts` and `preview_bundle`. Applying stays `Tx::import_bundle` (`importBundle`, MCP `import_bundle`).

## Why

Existing layers and bundle imports preserve evidence, but callers need a readable view of disagreement and a preview before importing another memory.

## What Changes

Add conflict inspection and dry-run bundle previews with explicit choices for subsequent writes.

## Capabilities

### New Capabilities

- `memory-conflict-review`: conflict inspection and import previews.

### Modified Capabilities

None in this draft. Implementation must add delta modifications if an existing public requirement must change.

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No replica-id allocation or distributed merge protocol.
- No automatic resolution or majority-vote belief writes.
- No unrequested UI implementation.
