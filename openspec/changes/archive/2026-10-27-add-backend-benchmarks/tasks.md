## Status

Implemented 2026-10-27. Depends on `add-native-runner` and `add-tm-cli`. See the Resolution in `design.md`.

## 1. Harness

- [x] 1.1 `bench/backends` crate (its own workspace, outside the default build) with tiramemsu (both backends, SQLite also with LFTJ) and Oxigraph 0.5 adapters; the shared harness is the workspace crate `tm-bench`.
- [x] 1.2 Seeded generators (WatDiv-style, uniform and hub-and-spoke graphs); query sets; churn through the write workloads.
- [x] 1.3 Driver with result equality checks before timing — `@tg: implements:: [[openspec:backend-benchmarks#Equal results before timing]]`.
- [x] 1.4 Workloads of the spec — `@tg: implements:: [[openspec:backend-benchmarks#Workload coverage]]`; results JSON with seeds and environment — `@tg: implements:: [[openspec:backend-benchmarks#Reproducible runs]]`.
- [x] 1.5 `tm bench` reduced suite (SQLite and redb).

## 2. Runs and report

- [x] 2.1 Runs at 10⁵ and 10⁶ (10⁷ not run: the 10⁶ run took 39 minutes); `bench/backends/README.md` with strengths and weaknesses — `@tg: implements:: [[openspec:backend-benchmarks#Honest report]]`.

## 3. Articles

- [x] 3.1 `article/redb-backend/two-engines.md` and `wcoj-on-kv.md`; site pages `site/articles/two-engines.html` and `wcoj-on-kv.html` and their cards. No figures: the numbers are in the text and the report's tables.

## 4. Tests and checks

- [x] 4.1 Smoke test at 10⁴ (`crates/tm-bench/tests/harness.rs`, `@tg: verifies::` the spec scenarios).
- [x] 4.2 lat.md `roadmap.md#Benchmarks`, `prior-art.md` (Oxigraph, Kùzu, Umbra, Free Join); `tg check`, `tg ontology check`, `tg trace backend-benchmarks --gaps`, `openspec validate add-backend-benchmarks`.
