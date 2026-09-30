## Dependencies

- M0 (`add-core-store`), `add-path-engine` (virtual hops `^sys:subject`/`^sys:object`, used only by the equivalence test) and `add-named-graphs` (memberships in bundles) are archived and are the base of this change.
- Other changes in flight edit the path engine, the predicate schema and SPARQL result provenance. This change touches none of those files except the facade `view.rs`, where its methods sit in a separate `impl` block.
- Test code references lat.md test specs with `// @lat: [[tests#…]]`, placed next to the covering test. Do not edit `lat.md/` until behaviour is implemented (group 7).

## 1. Dependents in `tm-core`

- [x] 1.1 Implement `read::dependents(exec, spec, root) -> Result<Vec<Eid>>`: root visibility check, then breadth-first `UNION` probes on `s` and `o` with a visited set, each expansion ordered by eid; every time predicate from `scan_predicates` (spec `statement-dependents` "Dependents of a statement", "Dependents follow the view").
- [x] 1.2 Unit and integration tests in `tm-core`: layers and references, plain nodes, cycle, not visible, retracted layers not walked, as-of of a since-retracted structure, history, valid-at, more than `max_cascade`. Both hosts (`host_test!`).
- [x] 1.3 Property test in `tm-core`: over random layered graphs with random retractions (the `props.rs` operation style), `dependents(e)` on the now view equals the retracted set plus retracted memberships of a dry-run `retract(e)`, for every live `e`.

> Notes (group 1): `read::dependents` builds one SQL text per call (`SELECT a.eid … a.s = ?1 AND <view> UNION SELECT a.eid … a.o = ?1 AND <view> ORDER BY eid`, `?1` reused by both halves) and rebinds `?1` per visited eid, so the per-probe cost is the cascade's. A crate-private `read::visible(exec, spec, eid)` checks the root under the same view predicate. Tests are `crates/tm-core/tests/statement_dependents.rs` (4 `host_test!`s on both hosts, plus the property test). The property test is stronger than the spec: besides set equality it checks that the dependents without memberships are exactly `TxReport.retracted` in report order, and it covers reference cycles (a statement that names the next eid), links between statements, memberships, confirmations and supersedes. 48 cases by default; 400 cases ran clean in 9 s.

## 2. Dependents in the facade and bridge

- [x] 2.1 `View::dependents(eid)` in a separate `impl` block of `crates/tiramemsu/src/view.rs`, with a doctest.
- [x] 2.2 Three-way property test in `crates/tiramemsu/tests/`: dependents, dry-run retraction, and the ends of `(^sys:subject|^sys:object)*` in `REACH` mode with `u32::MAX` hops, as sets, under the now view.
- [x] 2.3 JSON bridge read op `dependents` (`eid`, `view`); bridge test.

> Notes (group 2): the facade block is at the end of `view.rs`, after the main `impl`, marked with a comment naming this change. The three-way test is `crates/tiramemsu/tests/statement_dependents.rs` (`prop_dependents_match_cascade_and_path`, 32 cases by default, 300 ran clean); it compares only live statements, because the zero-length step of `*` yields the start term even when it is not visible. The same file checks the as-of and history views of a retracted structure and a speculation (`Db::with`). The bridge accepts `eid` as a number or `{"stmt": n}` and rejects a missing `eid` with `InvalidArgument` (`bindings/json/tests/bridge.rs`).

## 3. Bundle value and export in `tm-core`

- [x] 3.1 Module `bundle`: `Bundle`, `BundleStatement`, `BTerm`, `ImportReport`, `ImportedStatement`, re-exported from the crate root, with doc comments and a doctest.
- [x] 3.2 `read::bundle(exec, spec, root) -> Result<Bundle>`: dependents, downward closure, the three exclusions with propagation to a fixpoint and a recomputed closure, root errors (`NotLive`, `Unsupported`), term decoding, anonymous labels, topological order with eid ties and cycle members last.
- [x] 3.3 Tests: members, evidence carried downward, confirmation / supersede link / tx metadata exclusions, layer on an excluded statement, order, stability.

> Notes (group 3): the module is `crates/tm-core/src/bundle.rs` (types, `Bundle::check`, the crate-private `export` and `import_order`); `read::bundle` is a thin public wrapper so the reads stay together. `engine::reserved` became `pub(crate)` so the export applies exactly the user-write predicate rule (`check_predicate`), with `sys:inGraph` as the one exception. A root that falls under an exclusion fails with `Unsupported { feature }` carrying the reason ("bundle root that references a transaction", "... with the engine predicate ...", "... which is not in the view"). A bundle is built in one read snapshot and decodes terms through a per-call `TermReader`, so it also works inside `Db::with`. Tests: `crates/tm-core/tests/fact_bundles.rs` (`members_order_and_stability`, `exclusions`, both hosts).

## 4. Import in `tm-core`

- [x] 4.1 `Tx::import_bundle(&Bundle) -> Result<ImportReport>` in `engine/bundle.rs`: validate (ids, references, predicate IRIs, skolem values, root), reject cycles before writing, then assert / `add_to_graph` in order with fresh nodes per anonymous label.
- [x] 4.2 Tests: round trip between two databases (layers, nested layers, a belief pointing at the root, a membership, valid times), idempotent re-import, existing root fact, anonymous nodes, skolem value rejected, cycle rejected, schema violation atomic, as-of bundle of a since-retracted structure.

> Notes (group 4): import re-sorts topologically (ties by position), so a hand-written bundle in any order imports, and a bundle built by `read::bundle` imports in its own order; a self-reference counts as a cycle of one. Memberships go through `add_to_graph`, so a membership on a `sys:` statement or a non-node graph fails as it does anywhere. Found while testing: statements with equal content and overlapping valid time (parallel edges made with `create`) collapse into one eid on import, because assert matches them; this is now stated in design.md (Decision 8), the spec and `lat.md`. Beyond the listed tests, a property test (`random::prop_bundles_round_trip`, both hosts, 24 cases by default, 300 ran clean) checks that export, import into an empty file and export again give the same bundle, and that a second import is a no-op, for every live statement of random layered graphs. The round-trip test compares the target's own bundle of the imported root with the source bundle, which checks contents, layer structure, membership and valid times at once.

## 5. Serialization in the facade

- [x] 5.1 `BundleFormat` extension trait: `to_json`, `from_json` (format `tiramemsu-bundle/1`), `to_ntriples` over the `tm-sparql` term renderer and N-Triples writer.
- [x] 5.2 `View::bundle(root)` in the facade block, with a doctest.
- [x] 5.3 Tests: JSON round trip stable over every term kind, unknown version rejected, malformed statements rejected, N-Triples parses as RDF 1.2 (`oxttl`) and carries the reifier and annotation triples.

> Notes (group 5): `tm-core` depends only on `thiserror` and `lru`, so the forms are the facade trait `BundleFormat` (`crates/tiramemsu/src/bundle.rs`, implemented for `tm_core::Bundle`) plus the constant `BUNDLE_FORMAT`. The facade gains `serde_json` (already in the tree through `tm-cypher`) and the dev-dependency `oxttl` for the parse test. `to_json` returns a `serde_json::Value`, as `CypherResult::to_json` does. N-Triples reuses `tm_sparql::results::{term::render, nt::write}`; valid-time bounds are UTC `xsd:dateTime` literals on `tm:validFrom` / `tm:validTo`. `View::bundle` sits in the same separate `impl` block as `View::dependents`. Tests: `crates/tiramemsu/tests/fact_bundles.rs`; the JSON test's fixture had to use canonical literals (`12.5`, not `12.50`), which is what a bundle read from a database always holds.

## 6. Bundles in the JSON bridge

- [x] 6.1 Read op `bundle` (`eid`, `view`) and transaction op `importBundle` (`bundle`, `as`), documented in `Database::call`.
- [x] 6.2 Bridge test: read a bundle from one database, import it into another in a `transact` call, check the mapping and the `as` reference.

> Notes (group 6): `importBundle` reuses `Bundle::from_json`, so a malformed bundle fails with code `InvalidTerm` (a database error, not `InvalidArgument`). `as` on `importBundle` names the imported root. The Node and Python packages reach both ops through their generic `call`; typed wrapper methods are a follow-up, because their sources are outside this change.

## 7. Documentation and checks

- [x] 7.1 `lat.md/time-model.md` Cascade (read-only preview), `lat.md/data-model.md` (Fact Bundles section), `lat.md/api.md` (Rust surface, bindings).
- [x] 7.2 `lat.md/tests.md` sections for the new tests, each referenced by exactly one `@lat:` comment; `lat check` clean.
- [x] 7.3 `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `openspec validate add-fact-bundles --strict`.

> Notes (group 7): `lat.md/time-model.md` gained `Cascade#Dependents`, `lat.md/data-model.md` a `Fact Bundles` section with `Bundle Formats`, and `lat.md/api.md` and `lat.md/bindings.md` list the new methods and ops. `lat.md/tests.md` has a `Dependents` section (7 leaves) and a `Fact Bundles` section (14 leaves), each leaf referenced by one `@lat:` comment. `lat check` passes. Final run: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean, and `cargo test --workspace` passes 1047 tests with 0 failures (1019 after part A). The change is not archived.
