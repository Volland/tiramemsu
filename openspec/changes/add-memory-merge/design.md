## Context

`reserve-replica-id` split the payload of allocated ids into a 12-bit origin and a 48-bit counter, bounded the counters at 2⁴⁸ − 1, and made format 1 reject every non-zero origin (`lat.md/data-model.md#ObjectId#Origin Bits`). It deferred two questions: whether `TX` carries an origin (a gap-free local `t` conflicts with importing foreign transactions) and the skolem form for foreign ids (`<origin>.<n>` or a file UUID).

The event log is a view over `triple` (`lat.md/time-model.md#Event Log`; `crates/tm-core/src/event.rs`, `Db::events_since`): one assert row at `t_add` per statement and one retract row at `t_ret` with `ret_kind`. `asOf(t)` equals the replay of events up to `t`. Fact bundles move values with bundle-local ids and import with assert semantics; `Db::preview_bundle` runs an import as a dry run (`crates/tiramemsu/src/review.rs`). `View::conflicts` (`crates/tm-core/src/conflict.rs`) lists overlapping distinct values with attributed evidence.

## Goals / Non-Goals

**Goals:**
- One canonical name per statement across every file that holds it, without rewriting a stored id.
- A merge whose result does not depend on order or repetition.
- Incremental exchange with a preview before anything is written.

**Non-Goals:**
- Transport, discovery, conflict resolution by the store, merging derived state.

## Decisions

### D1. Origin 0 means "this file"; fleet numbers are assigned by the operator

Each file keeps its local ids at origin 0 forever (they cannot be rewritten). A file joins a fleet by taking a fleet number `meta.origin` = n once; from then on its canonical ids are `(n, counter)` everywhere else. On import, origin n maps back to 0 in file n, and every other origin is stored as is. The registry maps numbers to file UUIDs; a change set from an unknown UUID registers it, and a known number with a different UUID fails with `OriginConflict`, which catches two files configured with the same number.

*Alternatives:* a file UUID inside every id (no room in 60 bits); per-file local numbering of foreign origins (ids would differ across files, defeating one-name-everywhere); automatic number assignment (needs coordination the store does not have).

### D2. Skolem IRIs: `<origin>.<n>` for foreign, unchanged for local

Foreign ids render as `urn:tiramemsu:stmt:<origin>.<n>` (and `node:`, `bnode:`, `tx:`). Local ids keep `urn:tiramemsu:stmt:<n>`, so every IRI already stored, cited or sent stays valid; `stmt:<self>.<n>` parses to the same local id. The decimal form `stmt:281474976710661` (origin folded into the number) stays rejected, as `reserve-replica-id` specified.

*Alternatives:* UUID-based IRIs (long, and the UUID is already in the registry); always rendering local ids with the fleet number (breaks cited IRIs and changes output for single-file users).

### D3. `TX` origins occur only as values

The `tx` table and `t` stay local and gap-free; a merge is one local transaction with its own `t` and instant, and merged statements have `t_add` = that `t` (transaction time is when *this* file learned it). Foreign tx metadata arrives as statements about foreign `TX` ids (`(tx:2.41 sys:author :agent7)`), and confirmations keep foreign `TX` objects. The original transaction and instant per merged statement go into the derived table `merged_from`, so "when did the other agent learn this" stays answerable without a second time axis in `triple`.

*Alternatives:* importing foreign `tx` rows (breaks the gap-free `t` and the instant-to-`t` mapping of `asOf`); dropping origin time (loses provenance).

### D4. Union by canonical id, earliest retraction, content check

For each statement in a change set: unknown id → insert with its content and valid time (cardinality and uniqueness are not applied to other origins' statements); known id with equal content → nothing; known id with different content → `MergeConflict`, nothing written. For each reported retraction: locally live → retract in the merge transaction with the reported kind, and cascade local dependents with kind `cascade`; locally retracted → unchanged. Because inserts are by known id, references inside the change set (including cycles) are resolved before insertion. The resulting set of (canonical id, content, retracted) is the same for any order and number of merges, which a property test checks over three files.

*Alternatives:* assert semantics (merging would mint or deduplicate by content, so ids would diverge); latest-wins tombstones (not a CRDT; retraction is already once-only).

### D5. Cross-origin cardinality is a read policy

Enforcing `sys:one` or `sys:unique` during a merge would retract or reject another origin's statements depending on which file merged first. So a merge never applies them across origins; local writes keep enforcing them against the union (a local `sys:one` assert retracts overlapping values of any origin, and that retraction then propagates on the next sync). Violations created by merges are listed by `View::conflicts` with `cross_origin: true` and each statement's origin in its evidence. For callers that want one value, the view option `origin_precedence` hides, for `sys:one` predicates, every value whose origin ranks below another overlapping value's origin for the same subject; it is a visibility predicate emitted by `scan_predicates`, so every surface agrees.

*Alternatives:* pick a winner at merge time (order-dependent); never resolve (callers must post-process every read).

### D6. Change sets come from the event log

`export_changes(since_t)` reads the events with `since_t < t ≤ last_t` and emits, references first: every asserted statement with its canonical id, content, valid time and origin time (its own `t`/instant for local statements, `merged_from` for relayed ones), and every retraction with its kind. Statements of any origin are included, so changes relay through intermediate files. Statements referencing a statement outside the change set and outside the target are an error at merge (`NotLive` naming the missing id), not silently dropped; the exporter includes referenced statements not yet sent to that peer when a peer is named. `sync_peer` records `sent_to_t` and `received_to_t` per peer UUID, updated in the merge transaction and on explicit acknowledgement of an export.

### D7. Preview is a dry run of the real merge

`preview_merge` is `Tx::merge` inside `TxOptions { dry_run: true }`, with the same report structure as `preview_bundle`: proposed inserts, retractions and local cascades, schema failures reported rather than raised, origins that would be registered, and burned ids. Applying is an ordinary transaction that revalidates.

## Risks / Trade-offs

- [Two files with the same fleet number] → `OriginConflict` at the first merge that sees both UUIDs; numbers are fixed after the first export.
- [Foreign ids cost 8-byte varints] → Only merged rows pay; measured with the scale harness.
- [Cross-origin violations are visible as duplicates] → By design; `conflicts` and `origin_precedence` give the two ways to handle them.
- [Local layers on a statement superseded elsewhere are cascaded, not moved] → Reported in `MergeReport.cascaded` so the agent can re-attach them; moving them would need the other file's σ.
- [Large change sets hold the writer] → `merge` accepts chunked change sets; each chunk is one atomic transaction, and the cursor advances per chunk.

## Migration Plan

One storage format bump: creates `origin` (with the file's own UUID row), `merged_from`, `sync_peer` and `meta.origin = 0`, and changes no graph row. Files that never call `set_origin` behave exactly as before. Older builds refuse the migrated file. Bundles keep working unchanged and remain the tool for one-off hand-offs.

## Open Questions

- Should `export_changes` support a filter (graphs, predicates) for partial sharing, and how does that interact with relay completeness?
- Should `origin_precedence` also apply to `sys:unique` (hide lower-ranked subjects holding the same value)?
- Is 4 095 fleet numbers enough, or should a later format widen the origin field for files that never held local ids at high counters?
