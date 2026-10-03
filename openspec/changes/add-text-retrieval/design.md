## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Treat the index as derived storage and introduce term_fts through a format migration, never in format 1. Index dictionary text, then join candidate terms to statements using the central view predicates so retracted text does not leak into now results. Resolve a query in one snapshot. Return statement ids, text relevance, and separate evidence components; do not invent confidence for absent layers. Ranking policy and tokenizer configuration are explicit and versioned. JSON/Rust results and both language frontends share one logical retrieval operation.

Short strings can be encoded inline in ObjectIds without dictionary rows. Index those observed textual values as well as dictionary strings, using full encoded ObjectIds as derived-index identities. Decode inline strings into the derived index without inserting new dictionary terms. Preserve language-tagged string text and expose language metadata; define which typed literal datatypes are searchable before implementation.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

Implemented as follows.

- **Contract.** `View::text_search(&TextQuery) -> Result<Vec<TextHit>>` is the one logical recall (`tm_core::text::search`). `TextQuery { text, mode: All | Any | Phrase, graphs, predicates, limit, confidence }`; `TextHit { eid, s, p, o, text, lang, lexical, rank, evidence: TextEvidence { confidence: Option<f64>, confirmations, authors, t_add, added_at } }`. Query words are always quoted, so query text is never FTS5 syntax; a trailing `*` is a prefix.
- **Format migration.** Storage format 2. The 1→2 migration only inserts `meta.text_index = 0` (index layout version, 0 = never built) and `meta.text_stale = 0`; it runs on every host and touches no graph row. New files are created as format 1 and migrated, so both paths give the same schema. The bump makes format-1 builds refuse files whose index they would not maintain.
- **Opt-in index.** `term_fts` (FTS5, `text, lang UNINDEXED`, tokenizer `unicode61 remove_diacritics 2`, `rowid` = full ObjectId) is created only by `OpenOptions::text_index`, `Db::enable_text_index` or `Db::rebuild_text_index`, so default behavior and default write cost are unchanged. Once built, every writer with FTS5 indexes each new string object inside its transaction (speculations and dry runs roll back their index rows). The index holds every distinct string ever stored as an object: dictionary `STR`/`LANG_STR` and inline `SHORT_STR`, decoded without inserting a dictionary row. Typed literals are not searchable.
- **Hosts without FTS5.** They never issue FTS5 SQL; recall and index maintenance fail with `MissingCapability("fts5")` while every other operation works. If they write string statements to a file whose index exists, they record the first such eid in `meta.text_stale`; the next writer with FTS5 (at open or transaction start) indexes from there. Until then recall fails with the new `Error::TextIndexUnavailable` instead of silently missing hits (also used when the index was never built or has another layout version).
- **Visibility.** Matching values are joined to `triple` through `scan_predicates`, with graph memberships read in the same view, in one snapshot on the caller's connection.
- **Ranking** (`tiramemsu-text-rank/1`): negated bm25 descending, confidence descending (absent last), confirmations, distinct authors of the asserting and confirming transactions, `added_at` newer first, eid ascending. Confidence is the largest numeric object of `v:confidence` (or `TextQuery::confidence`) visible in the view, else `None`.
- **Entrypoints.** A new IR leaf `TextPattern` (eid, score, rank, confidence, limit, mode, view, graph `Any | Set`) compiles to the `tm_text(query, mode, view, graphs, limit)` connection table function, whose body is the same `text::search`. SPARQL groups `tm:textMatch`/`tm:textScore`/`tm:textRank`/`tm:textConfidence`/`tm:textLimit`/`tm:textMode` patterns on one subject variable into one `TextPattern` (`GRAPH <g>`, `FROM` and `SERVICE` time scopes apply; `GRAPH ?g` is rejected). Cypher's `CALL tiramemsu.text.search(query, {limit, mode, graphs})` lowers to the same leaf joined with the hit's statement and yields `statement, subject, predicate, text, score, rank, confidence`.
- **Budgets.** Recall is one budgeted operation: deadline and cancellation are polled per candidate and through SQL interruption, returned hits are charged as rows and bytes.
- **Rebuild.** `Db::rebuild_text_index` drops and refills `term_fts` from all statements in one write transaction, consuming no transaction number and changing no `triple`, `term` or `tx` row.
- **Deferred.** Vector indexes and embeddings (non-goals). Recall rows add no SPARQL query provenance. Evidence for every candidate is computed before the limit is applied, which is linear in the match count; no performance claim is made.
