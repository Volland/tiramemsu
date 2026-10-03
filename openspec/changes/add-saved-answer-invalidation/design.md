## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Store the query, parameters, view descriptor, result, supporting eids, provenance coverage, and checkpoint in versioned derived records. A current-view answer becomes stale when a dependency changes. Mark arbitrary mutable-view queries as requiring recheck for any potentially relevant insertion, including OPTIONAL additions; start with conservative all-event invalidation rather than claiming exact dependency completeness. Negative patterns, recursive paths, virtual predicates, volatile values, and clock-sensitive expressions require explicit coverage reasons. Fixed historical views can retain freshness for immutable graph data; external clock/volatile effects still require recheck. Only rerunning the query can advance its checkpoint and clear stale status. Event processing is replayable and does not rewrite graph history.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

- **Opt-in boundary:** the feature is a set of new `Db` methods; nothing changes for a database that never saves an answer, except that format 3 creates two empty tables.
- **Storage:** derived records in `saved_answer` (one row per name, JSON columns for params, view, settings, result and coverage, a `layout` version for the row shape) and `saved_answer_dep` (cited eids). They are created by the format 2 to 3 migration, written by `Store::derived_write` (one write transaction with no transaction number, no event, no counter change), and never touched by graph triggers. No graph row is read-modified or rewritten.
- **Identity:** the query text verbatim, the language (SPARQL `SELECT`/`ASK`, read-only Cypher), the Cypher parameters in a lossless tagged JSON form, the `ViewSpec` as given, and the database `@vocab` and prefix table read at save time. A refresh reuses all of them, so a later settings change does not alter re-evaluation. SPARQL with parameters, updates and `CONSTRUCT` are `Unsupported`; IR input is not offered (frontends already lower deterministically from text plus settings).
- **Dependencies and coverage:** SPARQL `SELECT` runs with query provenance and the union of row eids is the dependency set. Coverage reasons are explicit enum values: `mutableView`, `noProvenance`, `negativePattern`, `existsPattern`, `recursivePath`, `virtualPredicate`, `volatile`, `clock`. They come from an IR walk (for Cypher, of the IR queries the run executed, recorded by a tracing runner, plus volatile reads), the provenance gaps, a second SPARQL lowering at another instant (a differing IR means `NOW()` is used), and a conservative text scan for Cypher clock functions.
- **Fixed versus mutable:** a view is fixed when its transaction time is an as-of point at or before the head (`Tx(t)` with `t <= last_t`, or `Instant(ms)` with `ms <= last_instant`, since later instants are strictly larger). Now, history and future as-of points are mutable, and so is a query any of whose patterns has a mutable scope.
- **Invalidation rules:** all-event invalidation for mutable answers, as planned. Over the events in `(cursor, head]`: the first retraction of a cited eid makes the answer `stale` with that event; otherwise the first event of any kind (or a transaction without events, which covers volatile writes) makes a fresh answer `recheck`. Fixed answers ignore events; `clock` answers become `recheck` once the clock moved past their evaluation. Status only moves fresh to recheck to stale.
- **Checkpoints:** the checkpoint and the cursor start at the `last_t` read before evaluation, so a racing commit is re-processed rather than skipped. `check_saved_answers` updates cursor and marks for every answer in one write transaction and returns only the status changes, so processing is idempotent and restartable and every logical invalidation is reported once.
- **Refresh:** only a successful re-run sets `fresh`, replaces result, dependencies and coverage, advances checkpoint and cursor and increments `revision`. A failed one processes pending events, records the error, and leaves the old result, mark and checkpoint. `Error::SavedAnswerNotFound { name }` is the typed error for an unknown name.
- **Surfaces:** JSON bridge operations return results in the shape of the live `sparql`/`cypher` calls; Node and Python wrap them with typed records; the MCP server adds four tools, the three writing ones hidden in read-only mode.
- **Deferred:** exact relevance analysis (per-pattern dependency matching), background or push notification, and storing IR instead of text. The text scan for Cypher clock functions over-approximates (any `date(`/`datetime(` call counts).

