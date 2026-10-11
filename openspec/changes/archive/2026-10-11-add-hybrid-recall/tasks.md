## Status

Implemented (unreleased, after 0.4.0) for Rust, SPARQL, Cypher, the JSON bridge, MCP and the Python and Node bindings. Redesigned 2026-10-11; builds on `add-vector-recall` (implemented, host-neutral, no libSQL) and text recall. SPARQL and Cypher forms (group 5) are not implemented. Unchecked items below are open follow-ups.

## 1. Contract

- [x] 1.1 Finalise `RecallQuery`, `RecallHit`, policy id `tiramemsu-recall-rank/1`, defaults (`limit` 10, `depth` `max(4 × limit, 50)`, `expand` 0, `rrf_k` 60). No upper bounds beyond the query budget.

## 2. Core recall

- [x] 2.1 `tm_core::recall::recall`: run text and vector recall in one snapshot with `limit = depth`; fail on any unavailable requested channel; `InvalidQuery` without either channel.
- [x] 2.2 Reciprocal-rank fusion by eid, retaining per-channel score and rank; absent channels as `None`.
- [x] 2.3 Ordering with the shared evidence tie-breaks, then eid; `rank`; `limit`.
- [x] 2.4 Expansion: per fused hit, up to `expand` visible layer statements, then up to `expand` visible subject facts, in the view; score 0; `via`; no repeats; ranked after fused hits.
- [x] 2.4a Decided: expansion follows the hit's structure in the view and ignores the channels' graph and predicate filters, because layers (source, confidence) are rarely graph members; documented in `lat.md/query.md#Hybrid Recall`.
- [x] 2.5 Budget: one meter across channels, evidence and expansion.
- [x] 2.6 Facade `View::recall`.

## 3. Bridge, MCP and bindings

- [x] 3.1 JSON bridge read op `recall`.
- [x] 3.2 MCP read tool `recall` (`text`, `mode`, `model`, `vector`, `graphs`, `predicates`, `limit`, `expand`, `view`).
- [x] 3.3 Python `view.recall(text=None, model=, vector=, mode=, limit=, depth=, expand=, graphs=, predicates=)` returning `RecallHit`s; Node `View.recall({text, mode, model, vector, limit, depth, expand, graphs, predicates})`.

## 4. Tests and documentation

- [x] 4.1 Acceptance tests for every scenario in `specs/hybrid-recall`: fusion, single-channel hits, expansion with `via`, score 0 and the cap, as-of expansion, FTS5-less host, neither channel, unbuilt index, surface agreement and budgets (`vector_recall.rs`, `vector_properties.rs`, bridge, MCP and binding suites).
- [x] 4.2 Fusion property test (`vector_properties.rs`): recomputed scores and channel ranks match; single-channel recall keeps the standalone order.
- [x] 4.3 View and graph fixtures for fused and expanded statements (`hybrid_recall_honours_views_and_graphs`).
- [x] 4.4 Channel failures (no FTS5: `MissingCapability("fts5")`, vector-only still works; unknown model: `InvalidQuery`), surface agreement (SPARQL, Cypher) and budget tests.
- [x] 4.5 Latency benchmark (`benches/vector.rs`, `recall_<n>_384`): 10.0 / 90.7 ms at 10⁴ / 10⁵ for text + vector, 15.2 / 112.4 ms with `expand: 2`; recorded in `crates/tiramemsu/benches/README.md`.
- [x] 4.6 `lat.md/query.md` sections for hybrid recall and its query languages, `lat.md/api.md` MCP tool list (`vector_search`, `recall`, `unembedded`), roadmap status; `lat check` passes.

## 5. Query languages

- [x] 5.1 IR leaf `RecallPattern` and table function `tm_recall`; SPARQL `tm:recall*` patterns and Cypher `CALL tiramemsu.recall({...})`; agreement test against `View::recall` with expansion.

## 6. Validation

- [x] 6.1 Validate with `openspec validate add-hybrid-recall --strict`; archive only after the open items are resolved or moved.
