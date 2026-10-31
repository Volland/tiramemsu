## 1. Baseline

- [x] 1.1 Record the 2026-10-29 profile split (≈70 % `goal_rows`, ≈29 % `Engine::run`) and the current benchmark times as the baseline in `bench/datalog/README.md`.

## 2. Result path (R1)

- [x] 2.1 `goal_rows`: decode each distinct id of the surviving rows once into a `(Term, Value)` table, rank the distinct ids by `total_cmp` with equal values sharing a rank, stable-sort rows by `(ranks, interval start)` and clone output terms from the table.
- [x] 2.2 Test "Mixed values sort in value order": rows over IRIs, strings, integers, doubles, booleans, dates and invented nodes come back in the order a decoded-value sort gives; annotate with `@tg: verifies:: [[openspec:datalog-evaluation#Deterministic results#Mixed values sort in value order]]`.

## 3. Interval shortcuts and emit filtering (R2, R3)

- [x] 3.1 `IvSet`: every-instant shortcuts in `union`, `subtract` and `covers`; unit tests against the general algorithm.
- [x] 3.2 `Rel::insert`: return early when the row's set is already every instant.
- [x] 3.3 `Cx::emit`: drop a derivation whose head tuple already exists with a set covering the derived one.
- [x] 3.4 Test "Timeless and timed runs agree": always-valid history, atemporal and valid-time runs, with and without proofs, give the same tuples in the same order; annotate with `@tg: verifies:: [[openspec:datalog-evaluation#Deterministic results#Timeless and timed runs agree]]`.

## 4. Relation storage (R4)

- [x] 4.1 Add `hashbrown` to `tm-datalog`; replace `Rel::map` with a `HashTable<u32>` hashing tuple slices into `data`; keep `find`, `insert`, `lookup` and `for_rows` behaviour.

## 5. Join loop (R5, R6)

- [x] 5.1 `Stored`: resolve an atom's access (loaded predicate and index for its bound mask, or probe) once per rule variant and read through it; derived atoms pre-resolve their index per mask.
- [x] 5.2 Plan cache per `(rule, delta position)` within a stratum, re-planned when a relation the rule reads grows tenfold or becomes non-empty; explain lines recorded once per variant per stratum.
- [x] 5.3 The existing Datalog tests (`crates/tiramemsu/tests/datalog.rs`, `datalog_props.rs`, enforcement and materialization) pass unchanged, including "Parallel and sequential agree" and the provenance property test.

## 6. Phase times and benchmark (R7)

- [x] 6.1 `run_goal` measures evaluation and result time (`std::time::Instant` off wasm, 0 on wasm32); explain appends `time: evaluation <ms> ms` and `time: results <ms> ms`. Annotate `run_goal`'s explain path with `@tg: implements:: [[openspec:datalog-evaluation#Explain a program run]]`.
- [x] 6.2 Test "Explain shows phase times"; annotate with `@tg: verifies:: [[openspec:datalog-evaluation#Explain a program run#Explain shows phase times]]`.
- [x] 6.3 Annotate the rank sort with `@tg: implements:: [[openspec:datalog-evaluation#Deterministic results]] {since: "2026-10-29"}`.
- [x] 6.4 Benchmark prints the evaluation/result split for the closures; re-measure every workload, re-profile, and update `bench/datalog/README.md` with times and gate verdicts.

## 7. Docs and checks

- [x] 7.1 `lat.md/datalog.md`: update [[datalog#Sorted Columnar Relations]] (row table, plan cache as built, access paths per variant) and [[datalog#Gates And Oracles]] (new measurements); record R1 as a typed `Decision` section with `motivated_by:: [[constraints#Read Performance First]]`, and the change itself as a `Change` section in a new `lat.md/changes.md` cited by `changed_by` edges; keep the `-contradicts` debt on `Rel`.
- [x] 7.2 Trace the three new scenarios to their tests through `@tg: verifies::` edges (`lat.md/tests.md` keeps no Datalog sections; Datalog scenarios are traced through OpenSpec), and confirm `tg trace datalog-evaluation` lists them as verified.
- [x] 7.3 `tg check`, `tg ontology check` and `tg trace datalog-evaluation --gaps` show no new gaps; `cargo clippy` and `cargo test` for `tm-datalog` and the facade with `datalog` pass; `cargo check --target wasm32-unknown-unknown -p tm-datalog` builds.
