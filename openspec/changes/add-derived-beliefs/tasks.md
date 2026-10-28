## 1. Derive

- [ ] 1.1 Add `SYS_DERIVED_FROM` and make it engine-owned (`vocab.rs`, `engine/reserved.rs`).
- [ ] 1.2 `Tx::derive(s, p, o, valid, support)` in `engine/derive.rs` of both engines: support validation (non-empty, live, not self), idempotency only for the same live support set, `create` semantics otherwise, schema checks.
- [ ] 1.3 `Tx::upsert_derived(p, o, support)` in both engines.
- [ ] 1.4 Extend the cascade and `dependents` with the support rule in both engines, in the same breadth-first order (support-derived statements appended with the subject/object dependents of the same parent, ascending eid).
- [ ] 1.5 Supersede: drop derived statements reached through support (not the root) and their structural dependents; replay links when the derived statement is the root (this also covers widening).
- [ ] 1.6 Bundles keep `sys:derivedFrom` links; import writes derived statements through `derive`.
- [ ] 1.7 Annotate implementations with `@tg: implements:: [[openspec:derived-beliefs#<Requirement>]]`.

## 2. Surfaces

- [ ] 2.1 Facade `Tx::derive`, `Tx::upsert_derived`; JSON bridge `derive` (transaction op); Node and Python wrappers; MCP write tool `derive` (hidden in read-only mode, `additionalProperties: false`).

## 3. Tests

- [ ] 3.1 Retracting a support retracts the derived statement and its layers in the same transaction (kind `cascade`); retracting one of two supports retracts it; a parallel derivation with other support stays live.
- [ ] 3.2 Re-deriving with the same support is `Existing`; deriving over an independently asserted identical statement creates a parallel statement, and retracting the support leaves the independent one live.
- [ ] 3.3 `dependents(e)` includes derived statements; dependents = dry-run retraction.
- [ ] 3.4 Supersede of a support drops (does not replay) the derived statement; supersede of the derived statement keeps its support links (widening).
- [ ] 3.5 A saved answer citing a derived statement goes `Stale` with `supportRetracted` after its support is retracted.
- [ ] 3.6 Bundle round trip of a derived belief re-derives it in the target; a second import changes nothing.
- [ ] 3.7 Direct assert of `sys:derivedFrom` fails with `ReservedNamespace`; `max_cascade` counts support-cascaded statements.
- [ ] 3.8 `upsert_derived` scenarios; every scenario runs on SQLite and redb (oracle agreement).
- [ ] 3.9 Annotate each test with `@tg: verifies:: [[openspec:derived-beliefs#<Requirement>#<Scenario>]]` (and the modified cascade/dependents scenarios).

## 4. Documentation

- [ ] 4.1 `lat.md/time-model.md#Cascade` (support rule), `lat.md/data-model.md` (derived beliefs, list `derived-beliefs` in `openspec:` frontmatter), `lat.md/recipes.md` (summary that goes stale), paper remark and `lat.md/overview.md#Formal Model`.
- [ ] 4.2 Record D1–D10 as `Decision` sections in lat.md with typed edges.
- [ ] 4.3 Run `tg check`, `tg ontology check` and `tg trace derived-beliefs --gaps`; close every gap.
