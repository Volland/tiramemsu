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

- [ ] 3.1 Module `bundle`: `Bundle`, `BundleStatement`, `BTerm`, `ImportReport`, `ImportedStatement`, re-exported from the crate root, with doc comments and a doctest.
- [ ] 3.2 `read::bundle(exec, spec, root) -> Result<Bundle>`: dependents, downward closure, the three exclusions with propagation to a fixpoint and a recomputed closure, root errors (`NotLive`, `Unsupported`), term decoding, anonymous labels, topological order with eid ties and cycle members last.
- [ ] 3.3 Tests: members, evidence carried downward, confirmation / supersede link / tx metadata exclusions, layer on an excluded statement, order, stability.

## 4. Import in `tm-core`

- [ ] 4.1 `Tx::import_bundle(&Bundle) -> Result<ImportReport>` in `engine/bundle.rs`: validate (ids, references, predicate IRIs, skolem values, root), reject cycles before writing, then assert / `add_to_graph` in order with fresh nodes per anonymous label.
- [ ] 4.2 Tests: round trip between two databases (layers, nested layers, a belief pointing at the root, a membership, valid times), idempotent re-import, existing root fact, anonymous nodes, skolem value rejected, cycle rejected, schema violation atomic, as-of bundle of a since-retracted structure.

## 5. Serialization in the facade

- [ ] 5.1 `BundleFormat` extension trait: `to_json`, `from_json` (format `tiramemsu-bundle/1`), `to_ntriples` over the `tm-sparql` term renderer and N-Triples writer.
- [ ] 5.2 `View::bundle(root)` in the facade block, with a doctest.
- [ ] 5.3 Tests: JSON round trip stable over every term kind, unknown version rejected, malformed statements rejected, N-Triples parses as RDF 1.2 (`oxttl`) and carries the reifier and annotation triples.

## 6. Bundles in the JSON bridge

- [ ] 6.1 Read op `bundle` (`eid`, `view`) and transaction op `importBundle` (`bundle`, `as`), documented in `Database::call`.
- [ ] 6.2 Bridge test: read a bundle from one database, import it into another in a `transact` call, check the mapping and the `as` reference.

## 7. Documentation and checks

- [ ] 7.1 `lat.md/time-model.md` Cascade (read-only preview), `lat.md/data-model.md` (Fact Bundles section), `lat.md/api.md` (Rust surface, bindings).
- [ ] 7.2 `lat.md/tests.md` sections for the new tests, each referenced by exactly one `@lat:` comment; `lat check` clean.
- [ ] 7.3 `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `openspec validate add-fact-bundles --strict`.
