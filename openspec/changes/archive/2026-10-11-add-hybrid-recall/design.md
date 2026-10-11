## Context

Status: implemented (unreleased, after 0.4.0) for Rust, SPARQL, Cypher, the JSON bridge, MCP and the Python and Node bindings.

Two recall channels exist, each view-aware, each with its own deterministic policy and the same evidence components: text (`tm_core::text::search`, policy `tiramemsu-text-rank/1`, `lat.md/query.md#Text Recall`) and vector (`tm_core::vector::search`, `add-vector-recall`, policy `tiramemsu-vector-rank/1`, exact by default and host-neutral, `lat.md/query.md#Vector Recall`). Both read evidence (confidence, `sys:confirmedBy`, distinct authors, `t_add`, `added_at`) in the query's view. Budgets bound one operation across all of it (`lat.md/query.md#Query Budgets`).

`docs/feature-roadmap.md` (0.6) asks for one operation that fuses text and vector hits by reciprocal-rank fusion, adds a bounded neighbourhood per hit, ranks by the evidence the store holds, and never invents a score. The earlier version of this change depended on libSQL through `add-vector-recall`; since 2026-10-11 (libSQL decided against) vector recall is host-neutral, so hybrid recall runs on every host where its requested channels run.

## Goals / Non-Goals

**Goals**

- One recall call that agents can use as their default memory lookup, from every surface, with the same answer everywhere.
- Fusion that needs no calibration between bm25 and similarities and is reproducible from the reported per-channel ranks.
- Expansion bounded per hit, so cost is proportional to `limit × expand`.
- Every number in the result traceable to a channel or to the RRF formula.

**Non-Goals**

- Tuning weights per channel or per deployment; learned ranking.
- Multi-hop expansion or expansion by semantic similarity.
- Partial results when a requested channel fails.

## Decisions

### H1. Query and result shape

```text
RecallQuery {
  text:   Option<TextQuery>,      // as View::text_search
  vector: Option<VectorQuery>,    // as View::vector_search
  limit:  usize (default 10),     // fused hits returned
  depth:  usize (default max(4 × limit, 50)),  // candidates per channel
  expand: usize (default 0),      // layer statements and subject facts per hit; 0 = none
  rrf_k:  u32 (default 60),
}
RecallHit {
  eid, s, p, o,
  score: f64,                     // Σ 1/(rrf_k + channel rank); 0 for expanded statements
  rank: u64,                      // 1-based position in the returned list
  lexical: Option<f64>,  text_rank: Option<u64>,     // from text recall
  similarity: Option<f64>, vector_rank: Option<u64>, // from vector recall
  evidence,                       // as text and vector recall
  via: Option<Eid>,               // set on expanded statements: the hit they were reached from
}
View::recall(&RecallQuery) -> Result<Vec<RecallHit>>
```

At least one of `text` and `vector` is required (`InvalidQuery` otherwise). Each channel keeps its own filters (`graphs`, `predicates`, `confidence`); its own `limit` is replaced by `depth`. The bridge, MCP and the bindings apply one `graphs` and one `predicates` list to both channels. There are no upper bounds in the core beyond the caller's query budget.

### H2. Reciprocal-rank fusion over statements

Both channels run in the same snapshot with `limit = depth`, so their view filters, policies and ranks are exactly those of the standalone operations. For a statement `e`, `score(e) = Σ_{c ∈ channels returning e} 1 / (rrf_k + rank_c(e))`. A channel that did not return `e` contributes nothing and is reported as `None` for that hit, not as rank infinity or score zero.

Alternatives considered:

- **Score combination (CombSUM, CombMNZ, weighted sum).** Requires mapping bm25 and similarities onto one scale; any mapping is a calibration the store would be inventing, and it changes with the candidate set. Rejected.
- **Interleaving.** Ignores agreement between channels, the main signal hybrid recall exists to use. Rejected.
- **RRF** (Cormack, Clarke and Büttcher, SIGIR 2009) uses only ranks, rewards agreement, needs one constant and is reproducible from the reported ranks. Chosen; `rrf_k` is exposed for small memories.

### H3. Ordering, policy `tiramemsu-recall-rank/1`

Fused hits: `score` descending; then the evidence tie-breaks of vector recall (confidence descending with absent last; `sys:confirmedBy` count descending; distinct authors descending; `added_at` newer first); then eid ascending. `limit` cuts after ordering. Evidence is the shared function of text and vector recall, read in the query's view, and reported raw. Alternative considered: evidence as extra ranked lists inside RRF; rejected because it turns "how well supported" into "how relevant" with an arbitrary equal weight.

### H4. Bounded expansion

When `expand = n > 0`, for each returned fused hit in rank order:

- **Layers:** up to `n` visible statements whose subject is the hit's eid (its confidence, source, confirmation and other annotations).
- **Subject facts:** up to `n` visible statements whose subject is the hit's subject `s` (other facts about the same node), excluding the hit itself.
- Both use the query's view (its time predicates). They do not apply the channels' graph or predicate filters: an expanded statement is visible in the view but need not be a member of the requested graphs. Each expanded statement carries `via` = the hit's eid, `score` 0, per-channel fields `None`, and its evidence.
- Expanded statements are ranked after all fused hits, grouped by their `via` hit in rank order, layers before subject facts, each group in eid order (up to `n` taken per group before removing repeats). A statement already returned (as a fused hit or by an earlier expansion) is not repeated.

The first implementation keeps expansion to one step on two fixed edges, which covers what agents ask most ("what do we know about this fact and its subject") without the path engine. Multi-hop expansion through the path engine with completeness reporting is future work.

### H5. Channels must be available

If a requested channel cannot run (text on a host without FTS5 or with an unbuilt or unavailable text index; an unknown embedding model or an invalid vector), the call fails with that channel's typed error (`MissingCapability`, `TextIndexUnavailable`, `InvalidQuery`) and returns nothing. Silently fusing a single list would change the ranking contract without telling the caller. The vector channel needs no host capability.

### H6. Entrypoints

- **Core and facade.** `tm_core::recall::recall` (channels, fusion, ordering, evidence, expansion); `View::recall`.
- **Phase 1 (implemented).** JSON bridge read op `recall` (`text`, `mode`, `model`, `vector`, `limit`, `depth`, `expand`, `rrfK`, `graphs`, `predicates`, `prefilter`); MCP read tool `recall` (`text`, `mode`, `model`, `vector`, `graphs`, `predicates`, `limit`, `expand`, `view`); Python `view.recall(text=None, model=None, vector=None, mode="all", limit=None, depth=None, expand=0, graphs=None, predicates=None)` returning a list of `RecallHit` (text alone, vector alone or both); Node `View.recall({ text, mode, model, vector, limit, depth, expand, graphs, predicates })`. In Python and Node a model may be a plain IRI string.
- **Query languages.** SPARQL `tm:recall*` patterns and Cypher `CALL tiramemsu.recall({...})` compile through the IR leaf `RecallPattern` to the `tm_recall` table function, which runs `tm_core::recall::recall` on the calling connection, as `tm_vector` does for vector recall.
- **Budgets.** One operation: both channels, evidence and expansion draw on one meter; returned statements are charged as rows and bytes; a stop returns no partial result.

## Risks / Trade-offs

- **Cost.** Each channel reads evidence for its `depth` candidates, plus up to `2 × expand` statements per hit. No hybrid latency benchmark yet; the vector channel alone is measured in `crates/tiramemsu/benches/README.md` (about 1.2 µs per 768-dimension embedding, exact).
- **RRF flattens strong single-channel matches.** Accepted: documented, the per-channel ranks are reported, and a single-channel call is available.
- **Hub subjects** have many facts; `expand` caps what each hit adds.
- **Prefiltered vector channel.** If the caller sets a vector prefilter, fusion inherits its approximation; the vector channel's `exact` flag is not lost, because the caller chose it explicitly.

## Migration Plan

Additive: new API, bridge op, MCP tool and binding methods. No storage change and no change to text or vector recall results. Rollback is removing the surfaces.

## Open Questions

- Should expansion also include statements whose object is the hit's subject (incoming facts)?
- Should `rrf_k` default lower (for example 10) for small memories?
- A per-channel `required: false` to fuse what is available?

## Sources

- G. V. Cormack, C. L. A. Clarke, S. Büttcher, "Reciprocal Rank Fusion outperforms Condorcet and individual Rank Learning Methods", SIGIR 2009 (the RRF formula and `k = 60`).
