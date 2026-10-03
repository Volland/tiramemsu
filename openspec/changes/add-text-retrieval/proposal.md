# Text recall with evidence ranking

## Status

Implemented. Rust: `View::text_search(&TextQuery) -> Vec<TextHit>` (`TextMode`, `TextEvidence`, `text::RANK_POLICY`), `OpenOptions::text_index`, `Db::enable_text_index`, `Db::rebuild_text_index`, `Error::TextIndexUnavailable`; storage format 2 (`meta.text_index`, `meta.text_stale`, derived FTS5 table `term_fts`). SPARQL `?e tm:textMatch "words"` (+ `tm:textScore`, `tm:textRank`, `tm:textConfidence`, `tm:textLimit`, `tm:textMode`), Cypher `CALL tiramemsu.text.search(query, {limit, mode, graphs})`, both through the IR leaf `TextPattern` and the `tm_text` table function. JSON bridge `textSearch`, `rebuildTextIndex`, `enableTextIndex`, open option `textIndex`; Node `View.textSearch`, Python `View.text_search`.

## Why

Agents often know words from a memory rather than a graph pattern. Text recall should return statements and the evidence needed to assess them.

## What Changes

Add FTS5 recall over string terms, statement and graph filtering, deterministic evidence ranking, and query-language entrypoints.

## Capabilities

### New Capabilities

- `text-retrieval`: text recall with evidence ranking.

### Modified Capabilities

None in this draft. Implementation must add delta modifications if an existing public requirement must change.

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No vector index or embedding provider in the first release.
- No LLM summarization or automatic belief changes.
