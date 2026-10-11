## 1. Contract

- [ ] 1.1 Finalize `DbMetrics` field names, units and `layout` version; record the ceiling (10⁷ statements, 1 000 versions per hot key, 100 writes/s) and the gate margins in `design.md`.
- [ ] 1.2 Write the scale fixture specification (node count, versions per key, predicate skew, valid-time episodes) so the generator is deterministic from a seed.

## 2. Scale harness

- [ ] 2.1 Add the harness (`bench/scale/`): deterministic fixture builder through `ImportSession`, JSON output per measurement, row-count check before timing.
- [ ] 2.2 Shapes: point and 2-hop under now / as-of / valid-at; churn at 1, 10, 100, 1 000 versions per key; history view and `events_since` over deep keys; path shapes from `benches/path.rs`; supersede cascade sizes; bulk import throughput; sustained write rate with p50/p99 commit latency.
- [ ] 2.3 Capture `EXPLAIN QUERY PLAN`, page-cache hit/miss counters and rows examined per shape where the host provides them.
- [ ] 2.4 Storage size per statement and index overhead at 10⁶ and 10⁷ (decides partial `hist_*`).

## 3. History-depth diagnosis and fixes

- [ ] 3.1 Reproduce the as-of 30× and current-view regressions at 1 000 versions; attach plans and cache counters.
- [ ] 3.2 Resolve an instant-based `as_of` to a `t` once per read instead of once per pattern (`crates/tm-core/src/view.rs`, review L3).
- [ ] 3.3 Fix any index-choice problem found (query shape or statistics), with a plan test that fails on the old plan.
- [ ] 3.4 If a format change is required, write the follow-up proposal with the measurements instead of changing the format here.

## 4. Gates in CI

- [ ] 4.1 Add `.github/workflows/bench-nightly.yml`: nightly 10⁶ run (three repetitions), weekly 10⁷ run with a cached fixture, results uploaded as artifacts.
- [ ] 4.2 Add the comparison script and `bench/scale/baseline.json`; fail on > 20 % p50 regression of a gated metric, on a ratio target miss once gating, or on sustained writes below 100/s or p99 above 50 ms.
- [ ] 4.3 Add the release check to the publishing workflows (latest nightly gate must be green).
- [ ] 4.4 Regenerate `crates/tiramemsu/benches/README.md` from harness output.

## 5. oxilite head-to-head

- [ ] 5.1 Port `write-cost` and `as-of-latency` to run on both engines with equal result counts (`bench/oxilite/`).
- [ ] 5.2 Run at 80 000–400 000 triples, record results and machine, publish in `bench/oxilite/README.md` and the site article whatever the outcome.

## 6. Metrics

- [ ] 6.1 Counters in `tm-core` (`Stats` runs and elapsed time, `TermDict`/`TermReader` hits, misses, evictions, clears, commit count) and the facade (writer lock wait, reader pool waits and timeouts, maintenance timing).
- [ ] 6.2 `Db::metrics()` returning `DbMetrics`; keep `Db::optimize_runs()` consistent with it.
- [ ] 6.3 JSON bridge op `metrics`; Node `Db.metrics()`, Python `Db.metrics()` with type stubs.
- [ ] 6.4 MCP read-only tool `metrics`, listed in read-only mode, `additionalProperties: false`.

## 7. Tests

- [ ] 7.1 Metrics: counters advance as specified (commit, optimize after `optimize_every` commits, reader wait under a saturated pool, reader timeout, term-cache hit after a repeated lookup); snapshot takes no writer lock (taken while a writer transaction is open).
- [ ] 7.2 Bridge, Node and Python metrics tests; MCP test that `metrics` is listed and callable in read-only mode and rejects arguments.
- [ ] 7.3 Harness self-test at 10⁴ statements in the regular test suite (row checks pass, JSON schema valid) so the harness cannot rot between nightly runs.
- [ ] 7.4 Facade-overhead benchmark re-run with metrics enabled; difference recorded.

## 8. Documentation

- [ ] 8.1 `lat.md/overview.md#Open Inputs`: ceiling settled; `lat.md/roadmap.md#Benchmarks`: targets and gate; new `lat.md` section for operational metrics with code refs; `lat check`.
- [ ] 8.2 README Status section: link the latest gate results; validate OpenSpec.
