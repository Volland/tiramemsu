## 1. Prerequisite: interval algebra core

- [x] 1.1 Implement tasks 1.1 and 1.2 of `add-interval-algebra` (`Allen`, `Valid::relate`, `Valid::intersects` in `crates/tm-core/src/report.rs`, with its property test) and check them off in that change.

## 2. Crate and feature

- [ ] 2.1 New crate `crates/tm-datalog` (README as crate docs, `unsafe_code` forbidden, publishable) on `tm-core`, `tm-store`, `rustc-hash`; default feature `parallel`.
- [ ] 2.2 Facade feature `datalog = ["dep:tm-datalog"]`, off by default, independent of `exec`; `scripts/feature-matrix.sh` combo `datalog` asserting `tm-exec` absent; `cargo publish --dry-run -p tm-datalog` in CI.
- [ ] 2.3 `dl:` vocabulary (`urn:tiramemsu:dl:source`) in `crates/tm-core/src/vocab.rs`; error variants `DatalogParse`, `DatalogUnsafe`, `DatalogUnstratifiable`, `DatalogReserved`, `DatalogNotHorn`, `DatalogUnboundedRecursion`, `DatalogUnboundedInvention`, `DatalogTemporalOperatorInAtemporal`, `DatalogDuplicateConstraint`, `DatalogIncomplete`, `UnknownProgram`, `RuleLanguageConflict` with bridge codes.

## 3. Language (`datalog-language`)

- [ ] 3.1 Lexer and recursive-descent parser with spans: prefixes, rules, facts, goals, constraints, atoms with `@ E`, `not`, aggregates, temporal operators, `exists`, builtins, durations, comments.
- [ ] 3.2 Name resolution: prefixes, stored versus derived relations, reserved atoms, arity checks.
- [ ] 3.3 Analysis: safety, stratification (with cycle reporting), monotonicity per stratum, metric-operator recursion, invention recursion, Horn check for head operators, predicate sets; `ProgramInfo`.
- [ ] 3.4 Builtins: comparisons in value order, arithmetic on numbers, instants and durations, string builtins, `now()`; wrong kinds are false.
- [ ] 3.5 Annotate implementations with `@tg: implements:: [[openspec:datalog-language#<Requirement>]]`.

## 4. Kernel (`datalog-evaluation`)

- [ ] 4.1 Columnar relations with interval side column, lazy sorted indexes, sort-dedup-coalesce merge.
- [ ] 4.2 Store source: stored and reserved atoms over `ReadStore` in the view's scope (graphs, seeds plus hops), predicate pruning, probe versus load by the DL11 estimate.
- [ ] 4.3 Rule variants for semi-naive evaluation; greedy join order from every first atom; hash join, index lookup and leapfrog triejoin for cyclic bodies; re-plan on tenfold delta change.
- [ ] 4.4 Semi-naive loop per stratum with limits (`max_iterations`, `max_facts`, `max_fragments`, `max_new_values`) and `Incomplete` status; budget polling and cancellation.
- [ ] 4.5 Magic-set rewriting with per-stratum fallback and explain reasons; `demand = false`.
- [ ] 4.6 Provenance side array and proof resolution; `proof: true`.
- [ ] 4.7 Parallel morsels behind `parallel`; deterministic output order; sequential on wasm.
- [ ] 4.8 Explain: per stratum and variant the order, access, estimates, actual rows, iterations, demand status.
- [ ] 4.9 Value invention: run-local invented nodes keyed by rule and frontier, computed head values.
- [ ] 4.10 Annotate implementations with `@tg: implements:: [[openspec:datalog-evaluation#<Requirement>]]`.

## 5. Time (`datalog-temporal`)

- [ ] 5.1 Interval-set algebra (intersect, union, subtract, clip, coalesce) with open ends and saturating arithmetic.
- [ ] 5.2 Joins intersect, merges union, negation subtracts; `valid_at` views; `Atemporal`.
- [ ] 5.3 Metric operators `past`, `always_past`, `future`, `always_future`, `since`, `until` in bodies; `always_*` in heads.
- [ ] 5.4 Windows: required for metric recursion, clipping each iteration; optional otherwise.
- [ ] 5.5 Aggregates over time by endpoint sweep per group.
- [ ] 5.6 Allen builtins and `interval`, `start`, `end`, `duration` on `Valid::relate`.
- [ ] 5.7 Pinned `now()` from the view's instant.
- [ ] 5.8 Annotate implementations with `@tg: implements:: [[openspec:datalog-temporal#<Requirement>]]`.

## 6. Programs and constraints (`datalog-programs`, `datalog-constraints`)

- [ ] 6.1 Facade `crates/tiramemsu/src/datalog.rs`: `Program`, `DatalogOptions`, `View::datalog`, `View::datalog_reason`, `View::program_info`, `View::explain_datalog`.
- [ ] 6.2 `TxDatalog::store_program`: parse and analyse, assert or supersede, idempotent for equal text, `RuleLanguageConflict` against `n3:source`; and the reverse check in `TxN3::store_rules`.
- [ ] 6.3 Constraints: declarations, duplicate check, `View::check` with `only`, violations with bindings, eids and intervals.
- [ ] 6.4 Annotate implementations with `@tg: implements:: [[openspec:datalog-programs#<Requirement>]]`, `[[openspec:datalog-constraints#<Requirement>]]` and `[[openspec:optional-query-frontends#Optional Datalog feature]]`.

## 7. Surfaces

- [ ] 7.1 JSON bridge feature `datalog`: read ops `datalog`, `datalogReason`, `programInfo`, `explainDatalog`, `check`; tx op `storeProgram`; `Unsupported` without the feature.
- [ ] 7.2 Node, Python and wasm wrappers with typings and docs; enable `datalog` in each binding.
- [ ] 7.3 MCP tools `datalog_query`, `check_constraints` (read) and `store_program` (write, hidden in read-only mode, `additionalProperties: false`).
- [ ] 7.4 CLI `tm datalog`, `tm check`, `tm program put|show`.

## 8. Tests

- [ ] 8.1 Scenario tests for every scenario of the six delta specs, on SQLite and redb, each annotated `@tg: verifies:: [[openspec:<capability>#<Requirement>#<Scenario>]]` directly above the test.
- [ ] 8.2 Property: a brute-force pointwise evaluator (per instant, over the statements valid then) agrees with the interval engine on random facts, intervals and positive, negated, aggregated and metric programs.
- [ ] 8.3 Property: demand rewriting equals full evaluation; tracked equals untracked; parallel equals sequential; probe equals load (forced either way).
- [ ] 8.4 Differential against `tm-n3` on random Horn programs and histories (conclusions and intervals equal).
- [ ] 8.5 Binding smoke tests (Node, Python), MCP tool tests, CLI tests.

## 9. Benchmarks

- [ ] 9.1 `tm-bench` Datalog suite on SQLite and redb at 10⁵ and 10⁶ statements: transitive closure (chain, hub, small world), same-generation, RDFS/OWL 2 RL subset on WatDiv, a temporal LUBM-style DatalogMTL set; `tm-n3` on the same Horn programs.
- [ ] 9.2 Reference script for Soufflé (interpreter), Nemo and MeTeoR on exported facts; results and gate verdicts (≥ 10× `tm-n3`, ≤ 2× Soufflé interpreter on closure and same-generation) in `bench/datalog/README.md`.
- [ ] 9.3 Tune the probe-or-load threshold and morsel size from the results.

## 10. Documentation

- [ ] 10.1 `lat.md/datalog.md`: `openspec: [datalog-language, datalog-evaluation, datalog-temporal, datalog-programs, datalog-constraints]` frontmatter, status of DL1–DL12 and DL16–DL23 as implemented, links to code; constraints from design as typed edges (`constrains::`, `motivated_by::`).
- [ ] 10.2 `lat.md/architecture.md#Crates` (row for `tm-datalog`, cargo feature), `lat.md/bindings.md` (Datalog ops), `lat.md/api.md` (API, MCP, CLI), `lat.md/overview.md#Non-Goals` (DL1 implemented).
- [ ] 10.3 `tm-datalog/README.md` with doctests; facade README feature list; CHANGELOG; site page on Datalog and when to choose it over N3.
- [ ] 10.4 `tg check`, `tg ontology check` and `tg trace datalog-language datalog-evaluation datalog-temporal datalog-programs datalog-constraints --gaps` show no new gaps; `openspec validate add-datalog-engine` passes.
