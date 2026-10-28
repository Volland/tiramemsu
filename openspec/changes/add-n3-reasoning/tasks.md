## 1. Crate and dependency

- [x] 1.1 Workspace dependency `eyeron = { git = "https://github.com/eyereasoner/eyeron", rev = "c4adb8c15315f7ea0325c5b901989f19a14ccc78" }`; new crate `crates/tm-n3` (`publish = false`, README as crate docs, `unsafe_code` forbidden) on `tm-core` and `eyeron`.
- [x] 1.2 Facade feature `n3 = ["dep:tm-n3"]`, off by default, independent of `exec`; `scripts/feature-matrix.sh` combo `n3` (core + n3; asserts `tm-exec` absent).

## 2. Read-only reasoning (`n3-reasoning`)

- [x] 2.1 Term mapping Value ↔ eyeron `Term` through the RDF rendering (`urn:tiramemsu:node|bnode|stmt:N`), literals with datatype and language.
- [x] 2.2 Rule analysis: body predicates (forward, backward, nested formulas), variable-predicate detection, use of virtual predicates (`tm:content`, `tm:validFrom`, `tm:validTo`, `tm:now`, `tm:segmentFrom`, `tm:segmentTo`), `time:localTime` rewrite to `tm:run tm:now`.
- [x] 2.3 Scope loading in the facade (`src/n3.rs`): whole view, graph members, seeds with hops; pruning by predicate; virtual triples on demand.
- [x] 2.4 Pointwise sweep: segments from valid-time bounds inside the window, per-segment eyeron run, support keys, merge of adjacent segments with equal content and support; `valid_at` snapshot; `Atemporal`; `max_segments`.
- [x] 2.5 Pinned `now` (view instant), `tm:run` facts per run and segment.
- [x] 2.6 Proofs `{ rule_set, rule, premises }`, `proof_n3`, completion status from eyeron limits plus `Segments`, fuse as `contradiction`.
- [x] 2.7 `View::reason(rules, &ReasonOptions) -> Result<ReasonResult>`; errors `N3Parse`, `UnknownRuleSet`.
- [x] 2.8 Annotate implementations with `@tg: implements:: [[openspec:n3-reasoning#<Requirement>]]`.

## 3. Rule sets (`n3-rule-sets`)

- [x] 3.1 `n3:` vocabulary (`urn:tiramemsu:n3:source`, `derivedBy`, `rule`, `skolem`).
- [x] 3.2 `Tx::store_rules(rule_set, n3)`: parse, assert or supersede, idempotent for equal text; `Rules::Stored` read in the reasoning view.
- [x] 3.3 Monotonicity classification and `View::rule_set_info`.
- [x] 3.4 Annotate implementations with `@tg: implements:: [[openspec:n3-rule-sets#<Requirement>]]`.

## 4. Materialization (`n3-materialization`)

- [x] 4.1 `Db::materialize(rule_set, &MaterializeOptions) -> Result<MaterializeReport>`: snapshot reasoning excluding own conclusions, refusal of non-monotonic and clock-dependent rule sets, fuse abort, `require_complete`, `dry_run`.
- [x] 4.2 Commit in proof order through `derive` with premises + `R`; `n3:derivedBy` and `n3:rule` layers; `stale_support` skipping with dependents.
- [x] 4.3 Widening (hull supersede) and existing detection by content + support.
- [x] 4.4 Skolem nodes through `upsert_derived(n3:skolem, key, support)`.
- [x] 4.5 Annotate implementations with `@tg: implements:: [[openspec:n3-materialization#<Requirement>]]`.

## 5. Surfaces

- [ ] 5.1 JSON bridge feature `n3`: read op `reason`, tx op `storeRules`, db op `materialize`.
- [ ] 5.2 Node, Python, wasm wrappers (enable `n3` in each binding).
- [ ] 5.3 MCP tools `reason` (read), `store_rules`, `materialize` (write, hidden in read-only mode, `additionalProperties: false`).
- [ ] 5.4 CLI `tm reason`, `tm rules put|show`, `tm materialize`.

## 6. Tests

- [x] 6.1 Conformance: eyeron example corpus (facts loaded into a store, rules passed as text) gives eyeron's expected conclusions.
- [x] 6.2 Property: for random facts with random intervals and a fixed monotonic rule set, a conclusion of the sweep holds at random `t` iff it is concluded by the `valid_at(t)` snapshot.
- [x] 6.3 Property: after `materialize` and random retract/supersede sequences every live conclusion's support is live, and re-materializing reaches the same live set as a fresh materialization of a copy.
- [ ] 6.4 Scenario tests for every scenario in the four delta specs, on SQLite and redb where they write; each annotated `@tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]`.
- [ ] 6.5 `tm-bench` scenario: load, pruning and sweep cost against store size and segment count, SQLite and redb.

## 7. Documentation and site

- [ ] 7.1 `lat.md/reasoning.md` (typed `Decision`, `openspec: [n3-reasoning, n3-rule-sets, n3-materialization]`) with N1–N18 as typed sections and constraints; `lat.md/architecture.md#Crates`; N3 test spec sections in `lat.md/tests.md`.
- [ ] 7.2 `tm-n3/README.md` (crate docs, doctests); facade README feature list; CHANGELOG.
- [ ] 7.3 Site: a reasoning page and the N3 mini book under `site/n3/`, linked from the index and the feature matrix.
- [ ] 7.4 Run `tg check`, `tg ontology check` and `tg trace n3-reasoning --gaps` (and the other two capabilities); close every gap.
