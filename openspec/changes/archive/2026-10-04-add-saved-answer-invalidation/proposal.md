# Saved answers and conservative invalidation

## Status

Implemented. Final API: `Db::save_answer`, `saved_answer`, `saved_answers`, `check_saved_answers`, `refresh_answer` and `delete_saved_answer` (with `SavedQuery`, `SavedAnswer`, `AnswerStatus`, `CoverageReason`, `Invalidation`), the bridge operations `saveAnswer`, `savedAnswer`, `savedAnswers`, `checkSavedAnswers`, `refreshAnswer` and `deleteSavedAnswer`, their Node and Python wrappers, and the MCP tools `save_answer`, `saved_answers`, `check_answers` and `refresh_answer`. Storage format 3 adds the derived tables `saved_answer` and `saved_answer_dep`.

## Why

Positive statement provenance can identify stale answers, but cannot prove freshness for negative conditions, paths, or facts inserted after an answer was saved.

## What Changes

Add opt-in saved queries/results, dependency coverage metadata, event cursors, and conservative stale-answer notifications.

## Capabilities

### New Capabilities

- `saved-answer-invalidation`: saved answers and conservative invalidation.

### Modified Capabilities

None in this draft. Implementation must add delta modifications if an existing public requirement must change.

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No automatic edits to stored beliefs.
- No exactly complete why-not/path provenance implementation.
- No background LLM regeneration.
