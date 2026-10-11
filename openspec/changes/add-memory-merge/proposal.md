# Memory merge over origin bits

Target release: **0.9 — Many agents**. Builds on `reserve-replica-id` (archived 2026-10-01, implemented); source: `docs/feature-roadmap.md#0.9`. Signing is split into the small change `add-signed-bundles`, which this change can use but does not require.

## Why

Tiramemsu is one SQLite file per agent, and agents need to combine what they learned: two files that learned different things, or one file that was copied and diverged. The data model is already a CRDT. Statement content never changes, eids are never reused, and a retraction only sets `t_ret` once, from NULL, so statements form a grow-only set with a once-only tombstone (a 2P-set): a merge is the union of statements plus the earliest retraction of each.

The obstacle was identity, and `reserve-replica-id` removed it: the payload of `NODE`, `BNODE`, `STMT` and `TX` ids is `(origin << 48) | counter` (`crates/tm-core/src/id.rs`, `ObjectId::origin`, `ObjectId::counter`), every file writes origin 0, and any non-zero origin is rejected today (`ObjectId::check_origin`, applied to skolem IRIs in `crates/tm-core/src/codec.rs#parse_skolem`, to `Value`s, write operations, read lookups and bundle import). The reservation's design left two questions to this change: whether `TX` carries an origin, and the skolem form for foreign ids.

Fact bundles (`crates/tm-core/src/bundle.rs`, `crates/tm-core/src/engine/bundle.rs`) already move one belief with its evidence, by minting fresh local ids. That is right for hand-offs and wrong for synchronization: a statement cited by its skolem IRI in one file names a different statement in the other, and importing twice mints twice.

## What Changes

- **Origin registry.** A storage format bump adds `origin(n INTEGER PRIMARY KEY, uuid TEXT NOT NULL UNIQUE, label TEXT, registered_t INTEGER NOT NULL)` and the integer key `meta.origin` (this file's fleet number, 0 until set). Each file gets a UUID at migration (its own registry row). `Db::set_origin(n)` (1 ≤ n ≤ 4 095) joins a fleet once; it cannot change after the first export.
- **Foreign ids keep their identity.** Local ids stay origin 0 on disk (same bytes, same varint size); origin 0 means "this file", so a local statement's canonical id elsewhere is `(meta.origin, counter)`. A foreign statement, node or blank node is stored with its own origin, and an incoming id whose origin equals `meta.origin` maps back to origin 0. Format-gated: files of this format accept non-zero origins that are registered and differ from their own; allocation stays origin 0.
- **Skolem form.** Foreign ids render and parse as `urn:tiramemsu:stmt:<origin>.<n>` (likewise `node:`, `bnode:`, `tx:`). Local ids keep `urn:tiramemsu:stmt:<n>`, so every IRI already cited keeps resolving; `stmt:<self>.<n>` also parses as local.
- **Transactions stay local.** The `tx` table keeps a gap-free local `t`; a merge is one local transaction. Foreign transaction ids occur only as values (tx metadata subjects, `sys:confirmedBy` objects), with origin, which answers the reservation's open question. The original transaction and instant of each merged statement are kept in a derived table `merged_from(eid, origin, origin_t, origin_instant)`.
- **Merge semantics.** `Tx::merge(&ChangeSet)` inserts every statement whose canonical id is unknown (with its content and valid time, references resolved within the change set, so reference cycles merge too), and retracts every locally live statement the change set reports retracted, with the reported kind; local dependents then cascade as usual. A statement retracted locally stays retracted. Same id with different content fails the whole merge with `MergeConflict`. The result is independent of merge order (union plus earliest retraction).
- **Cross-origin policies.** Write-time schema checks still run for local writes against the union of all origins. A merge never enforces `sys:one` or `sys:unique` against another origin's statements (that would make merge order matter); violations surface as conflicts: `View::conflicts` gains a `cross_origin` filter and reports each statement's origin as evidence. An opt-in view option `origin_precedence: [origin…]` resolves `sys:one` predicates at read time, keeping per subject only the overlapping values from the highest-precedence origin.
- **Incremental sync.** `Db::export_changes(since_t)` returns a `ChangeSet` (`tiramemsu-changeset/1` JSON): the statements asserted or retracted in this file's local range `(since_t, last_t]`, of any origin, in canonical ids, plus the source's UUID and origin and the range. A derived table `sync_peer(uuid, received_to_t, sent_to_t)` keeps cursors per peer. `Db::preview_merge(&ChangeSet)` runs the merge as a dry run (like `preview_bundle`) and reports proposed inserts, retractions, local cascades, conflicts with local schema, newly registered origins and burned ids; applying is a separate ordinary transaction that revalidates.
- **Surfaces:** JSON bridge `exportChanges`, `previewMerge` (reads) and `merge` (transaction op), `setOrigin`; Node and Python wrappers. No MCP tool in this change (merging is an operator action).

## Capabilities

### New Capabilities

- `memory-merge`: origin registry, foreign ids, skolem forms, change sets, merge semantics, preview, sync cursors and cross-origin read policies.

### Modified Capabilities

- `object-encoding`: "ObjectId layout" accepts registered foreign origins in files of the merge format.

## Impact

- **`crates/tm-core`:** `src/id.rs` (`check_origin` becomes format- and registry-aware; canonical id mapping), `src/codec.rs` (`parse_skolem`/`skolem_iri` with `<origin>.<n>`), `src/value.rs` (rendering), new `src/merge.rs` (`ChangeSet`, export from the event log through `scan_predicates`, merge planning), new `src/engine/merge.rs` (`Tx::merge`, earliest-retraction rule, cascade, `MergeConflict`), `src/conflict.rs` (`ConflictQuery.cross_origin`, origin evidence), `src/view.rs` (`origin_precedence` visibility predicate for `sys:one` predicates, still produced by `scan_predicates`), `src/storage/` (format bump: `origin`, `merged_from`, `sync_peer`, `meta.origin`), `src/error.rs` (`MergeConflict`, `OriginConflict`, `OriginUnset`).
- **Facade (`crates/tiramemsu`):** `src/db.rs` (`set_origin`, `export_changes`), `src/review.rs` (`preview_merge`, next to `Db::preview_bundle`), a `ChangeSetFormat` like `BundleFormat` in `src/bundle.rs`.
- **Query front ends:** skolem parsing in `crates/tm-sparql/src/terms.rs` and Cypher `@id` resolution (`crates/tm-core/src/mapping.rs#Vocab#resolve_id`) accept the foreign forms.
- **Bindings:** `bindings/json/src/{read,tx}.rs`, Node, Python.
- **Storage size:** foreign ids are 8-byte varints (about +5 bytes per occurrence over the row and its nine index entries, as `reserve-replica-id` estimated); measured with the `add-scale-benchmarks` harness.
- **Docs:** `lat.md/data-model.md#ObjectId#Origin Bits`, `#Nodes and Identity`, `#Fact Bundles` (bundles versus change sets), `lat.md/time-model.md#Event Log`, `lat.md/storage.md`.

## Non-goals

- No network transport, discovery or automatic replication; change sets are values the host moves.
- No merging of volatile values, saved answers or derived indexes (they are rebuilt or caught up locally).
- No merging of sealed values across files (change sets withhold sealed statements, as bundles do under `add-crypto-shredding`).
- No moving of local layers when another origin superseded the statement they annotate (they cascade and are reported).
