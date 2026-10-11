## Dependencies

- `reserve-replica-id` (archived, implemented).
- Optional: `add-signed-bundles` for signed change sets. If `add-crypto-shredding` is archived first, rebase the `object-encoding` delta of this change on its "ObjectId layout" text.

## 1. Format and registry

- [ ] 1.1 Storage format bump: `origin` table with the file's own UUID row, `merged_from(eid, origin, origin_t, origin_instant)`, `sync_peer(uuid, received_to_t, sent_to_t)`, `meta.origin = 0`; no graph row changes; older builds refuse the file.
- [ ] 1.2 `Db::set_origin(n)` (1–4 095, once, fixed after the first export); `OriginConflict`, `OriginUnset` errors.

## 2. Identity

- [ ] 2.1 Format- and registry-aware `ObjectId::check_origin`; canonical id mapping (local 0 ↔ fleet number) on export and import.
- [ ] 2.2 Skolem render and parse of `<origin>.<n>` for `stmt:`, `node:`, `bnode:`, `tx:` in `crates/tm-core/src/codec.rs`, SPARQL terms and Cypher `@id`; local forms unchanged; `stmt:<self>.<n>` parses as local.

## 3. Change sets and merge

- [ ] 3.1 `ChangeSet` and `export_changes(since_t)` from the event log, references first, with origin times; JSON form `tiramemsu-changeset/1` (`ChangeSetFormat`).
- [ ] 3.2 `Tx::merge`: union by canonical id, content check (`MergeConflict`), earliest retraction with reported kind, local cascade, no cross-origin cardinality or uniqueness; `merged_from` rows; `sync_peer` cursor update; chunked application.
- [ ] 3.3 `Db::preview_merge` as a dry run with a `MergePreview` report.
- [ ] 3.4 Withhold sealed statements from change sets when sealing is present.

## 4. Cross-origin policies

- [ ] 4.1 `ConflictQuery.cross_origin` and origin evidence in `View::conflicts`.
- [ ] 4.2 View option `origin_precedence` for `sys:one` predicates, emitted by `scan_predicates`, honored by every surface.

## 5. Bindings

- [ ] 5.1 JSON bridge `setOrigin`, `exportChanges`, `previewMerge`, `merge`; Node and Python wrappers.

## 6. Tests

- [ ] 6.1 Property test over three files with random asserts, retractions, supersedes and merges in random orders: the final sets of (canonical id, content, retracted) are equal; merging twice changes nothing.
- [ ] 6.2 Identity: a statement cited as `urn:tiramemsu:stmt:2.5` resolves to the same content in every file holding it; a local `stmt:<n>` IRI cited before joining a fleet still resolves after.
- [ ] 6.3 Retraction from another origin cascades local layers (kind `cascade`) and is reported; a locally retracted statement stays retracted when the change set has it live.
- [ ] 6.4 Same id with different content fails with `MergeConflict` and writes nothing; two files with the same fleet number give `OriginConflict`.
- [ ] 6.5 Cross-origin `sys:one` conflict is listed by `View::conflicts` with origins; `origin_precedence` shows one value; local `sys:one` assert retracts a foreign value and the retraction syncs back.
- [ ] 6.6 Reference cycles merge; `merged_from` gives the original transaction and instant; preview reports exactly what the merge then does on an unchanged file.
- [ ] 6.7 Storage size of a merged store measured with the scale harness.

## 7. Documentation

- [ ] 7.1 `lat.md/data-model.md#ObjectId#Origin Bits`, `#Nodes and Identity`, `#Fact Bundles`, `lat.md/time-model.md#Event Log`, `lat.md/storage.md`; a decision row in `lat.md/overview.md` closing the two open questions of `reserve-replica-id`; `lat check`; validate OpenSpec.
