# Scale ceiling, benchmark gates and operational metrics

Target release: **0.5 — Evidence at scale**. Closes review finding H1 (`docs/project-review.md`); source: `docs/feature-roadmap.md#0.5`.

## Why

For a never-forget store, history depth is the workload, and the numbers that say whether it holds up are missing. `lat.md/overview.md#Open Inputs` still lists the scale ceiling as unknown, so every target in `lat.md/roadmap.md#Benchmarks` is unset. The last churn measurement (2026-10-03) showed as-of lookups going from 2.75 µs to 81.5 µs between 1 and 1 000 versions per key (about 30×), against a target of ≥ 70 % of the no-history baseline, and the current view also regressed at 1 000 versions with the cause unisolated. Nothing has re-measured it since; `crates/tiramemsu/benches/README.md` says its path table is an older run, and CI only checks that the benchmarks compile (`cargo bench --no-run`). The oxilite `write-cost` and `as-of-latency` ports listed in the roadmap were never run.

Operators also have no view into the engine at run time: `Db::optimize_runs()` and `Db::term_cache_len()` exist as test hooks, but maintenance time, reader-pool waits and cache hit rates are invisible, so a slow deployment cannot be diagnosed without a profiler.

## What Changes

- **Declare the scale ceiling** as a documented, tested input: 10⁷ statements per file, 1 000 versions per hot key, and 100 single-statement write transactions per second sustained, on a laptop-class machine. This settles the last open input of `lat.md/overview.md`.
- **A scale harness** (`bench/scale/`, a release-mode example that emits JSON) that builds churned and history-deep fixtures at 10⁶ and 10⁷ statements and measures: point and 2-hop latency under now, as-of and valid-at views; churn (1, 10, 100, 1 000 versions per key) for now and as-of; history depth (`history` view and `events_since` over deep keys); path shapes; supersede-cascade cost; bulk import throughput; sustained write rate with p50/p99 commit latency. Every measurement checks its result count before it is timed.
- **Isolate the history-depth regressions** (as-of 30×, current view at 1 000 versions) with `EXPLAIN QUERY PLAN` capture and page-cache counters in the harness output, and fix what does not need a format change (index choice, query shape, `as_of` instant resolution, see L3). A cold-history table or partial `hist_*` indexes are only decided here, and if needed get their own change with a format bump.
- **Nightly benchmark gates in CI**: a scheduled workflow runs the harness at 10⁶ statements, compares each gated metric with a checked-in baseline, and fails when one regresses by more than the agreed margin; a weekly run at 10⁷ records ceiling evidence. A release is blocked while the latest nightly gate is red. `crates/tiramemsu/benches/README.md` is regenerated from the harness output.
- **oxilite head-to-head**: port oxilite's `write-cost` (rows written per triple) and `as-of-latency` (present, past, depth; 80 000–400 000 triples) harnesses to run on both engines with equal result counts, and publish the result either way.
- **`Db::metrics()`**: a cheap, lock-light snapshot of counters kept since open — writer commits and lock wait, maintenance (`PRAGMA optimize` and `ANALYZE` runs and time, text-index catch-up time), reader-pool acquisitions, wait time and timeouts, term-cache hits, misses and evictions for the writer dictionary and the reader cache, and a few storage facts (page count, freelist pages, format version, `last_t`). Exposed on the JSON bridge (`metrics`), Node (`Db.metrics()`), Python (`Db.metrics()`) and as a read-only MCP tool `metrics`.

## Capabilities

### New Capabilities

- `scale-evidence`: the declared scale ceiling, the scale harness, history-depth benchmarks, nightly benchmark gates and the oxilite head-to-head.
- `operational-metrics`: the `Db::metrics()` snapshot and its JSON form.

### Modified Capabilities

- `mcp-memory-adapter`: adds the read-only `metrics` tool (ADDED requirement; no existing requirement changes).

## Impact

- **Facade (`crates/tiramemsu`):** `src/db.rs` (new `Db::metrics`, counters around `lock`/`lock_bounded`, `optimize`, `enable_text_index`/`rebuild_text_index`), `src/pool.rs` (`ReaderPool::acquire` wait and timeout counters), a new `src/metrics.rs` with `DbMetrics`. `Db::optimize_runs()` stays as a convenience over the snapshot.
- **Core (`crates/tm-core`):** `src/storage/stats.rs` (`Stats` records `ANALYZE`/`optimize` counts and elapsed time), `src/term.rs` (`TermDict` and `TermReader` hit, miss, eviction and clear counters; no behavior change to the clear-on-overflow policy, which the 0.4.x LRU item replaces), `src/engine/mod.rs` (commit counter, exposes `Stats`). Counters are plain integers updated under locks already held; `tm-core` stays free of a clock dependency by taking elapsed times from the facade's `Clock`.
- **Bindings:** `bindings/json/src/read.rs` (`metrics` op), `bindings/node/lib/index.ts`, `bindings/python/python/tiramemsu/_db.py`.
- **MCP (`crates/tiramemsu-mcp`):** `src/tools.rs` (new read-only tool `metrics`, listed in read-only mode).
- **Benchmarks:** new `bench/scale/` harness (release example under `crates/tiramemsu/examples/scale.rs` or a `bench/scale` crate outside the workspace default members), `bench/scale/baseline.json`, `bench/oxilite/` ports; existing `crates/tiramemsu/benches/{core,sparql,path}.rs` stay as criterion micro-benchmarks.
- **CI:** new `.github/workflows/bench-nightly.yml` (scheduled, 10⁶ gate nightly, 10⁷ weekly) and a release check in the publishing workflows (`npm-publish.yml`, `python-wheels.yml` and the crates release job) that requires a green latest gate.
- **Docs:** `lat.md/overview.md#Open Inputs` (ceiling settled), `lat.md/roadmap.md#Benchmarks` (targets), `crates/tiramemsu/benches/README.md`, `site/articles/tiramemsu-vs-oxilite.html` (head-to-head result).
- **Not affected:** storage format, query semantics, existing public signatures. No format bump.

## Non-goals

- No change to storage layout in this change (cold history, partial or clustered history indexes need their own change if the measurements call for them).
- No metrics exporter (Prometheus, OpenTelemetry); the snapshot is a value the host can export.
- No performance claim without matching result counts on representative data.
