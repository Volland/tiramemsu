# Hybrid recall with graph expansion and evidence ranking

## Status

Implemented (unreleased, after 0.4.0) for Rust, SPARQL, Cypher, the JSON bridge, MCP and the Python and Node bindings. Proposed 2026-10-10, redesigned 2026-10-11. Depends on `add-text-retrieval` (archived) and `add-vector-recall` (host-neutral exact vector recall; no libSQL dependency). Part of 0.6 "Agent recall" in `docs/feature-roadmap.md`.

## Why

An agent's real recall loop is: find candidates by words and by meaning, look at what surrounds them, and prefer what is better supported. Today an agent must call text recall and vector recall separately, merge two incomparable scores itself (bm25 and a similarity), then look up each hit's annotations and neighbouring facts. Done by hand this is slow, every agent does it differently, and the merged score is usually invented. The store already has every ingredient; one operation should combine them with a documented, reproducible policy.

## What Changes

- **`View::recall(&RecallQuery) -> Vec<RecallHit>`**: one operation over a text query, a vector query, or both, in one snapshot and one budget. `RecallQuery { text: Option<TextQuery>, vector: Option<VectorQuery>, limit (default 10), depth (candidates per channel, default max(4 × limit, 50)), expand: usize (default 0), rrf_k: u32 (default 60) }`.
- **Fusion by reciprocal rank (RRF).** Each channel runs its standalone recall with `depth` as its limit; a statement's `score` is `Σ 1/(rrf_k + rank)` over the channels that returned it. No normalisation of bm25 or similarities, no weights.
- **Evidence ordering.** Policy `tiramemsu-recall-rank/1`: RRF score descending, then the evidence tie-breaks of text and vector recall (confidence with absent last, confirmations, distinct authors, `added_at` newer first, eid ascending).
- **Hits report their sources.** `RecallHit { eid, s, p, o, score, rank, lexical, similarity, text_rank, vector_rank, evidence, via }`: per-channel score and rank are `None` when that channel did not return the hit.
- **Bounded expansion.** With `expand = n > 0`, each fused hit in rank order adds up to `n` of its visible layer statements (subject = the hit's eid), then up to `n` visible facts about its subject node (subject = `hit.s`), ranked after the fused hits with `score` 0 and `via` = the hit they were reached from; a statement is never repeated.
- **Errors.** A recall with neither channel is `InvalidQuery`; a text channel needs the text index (`TextIndexUnavailable` otherwise), a vector-only recall does not.
- **Surfaces (phase 1, implemented).** JSON bridge read op `recall` (`text`, `mode`, `model`, `vector`, `limit`, `depth`, `expand`, `rrfK`, `graphs`, `predicates`, `prefilter`), MCP read tool `recall`, Python `view.recall(text=None, model=, vector=, mode=, limit=, depth=, expand=, graphs=, predicates=)` returning a list of `RecallHit`, Node `View.recall({text, mode, model, vector, limit, depth, expand, graphs, predicates})`.
- **Query languages.** IR leaf `RecallPattern` compiled to the `tm_recall(text, mode, model, vector, view, graphs, limit, expand)` table function; SPARQL `tm:recallText`, `tm:recallMode`, `tm:recallVector`, `tm:recallModel`, `tm:recallScore`, `tm:recallRank`, `tm:recallVia`, `tm:recallLimit`, `tm:recallExpand`; Cypher `CALL tiramemsu.recall({text, model, vector, mode, limit, expand, graphs}) YIELD statement, subject, predicate, object, score, rank, via`.

## Capabilities

### New Capabilities

- `hybrid-recall`: fused text and vector recall, evidence ordering, bounded expansion and its entrypoints.

### Modified Capabilities

None. Text and vector recall keep their own contracts; hybrid recall calls them.

## Impact

`tm-core` (a `recall` module, `tm_core::recall::recall`, running both channels and the shared evidence, fusion, expansion), facade (`View::recall`, `RecallQuery`, `RecallHit`), JSON bridge, MCP server, Python and Node bindings, tests and `lat.md`; later `tm-ir`, `tm-sparql`, `tm-cypher`. No storage-format change and no host capability beyond what a requested text channel needs (FTS5).

## Non-goals

- No learned or weighted fusion, no reranking model, no LLM summarisation.
- No multi-hop or path-engine expansion in this change (one step: layers and subject facts).
- No salience or decay scores (a separate 0.6 item).
- No new index; no writes.
