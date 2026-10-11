## Context

The benchmarks today are criterion micro-benchmarks (`crates/tiramemsu/benches/core.rs`: churn, supersede, facade overhead; `sparql.rs`: point and 2-hop through SPARQL at `SPARQL_BENCH_STATEMENTS`, default 10⁵; `path.rs`: path shapes at `PATH_BENCH_STATEMENTS`, default 10⁵) plus one-off studies under `bench/` (`triangles`, `engine-comparison`, `named-graphs`, `eid-vs-reifier`). CI runs `cargo bench --no-run` only. The storage size baseline (about 153 bytes per statement, indexes 5.3× the table) comes from `lat.md/storage.md#Measured Footprint`.

The engine already counts one thing: `Stats::runs()` in `crates/tm-core/src/storage/stats.rs`, exposed as `Db::optimize_runs()`. The reader pool (`crates/tiramemsu/src/pool.rs`) waits on a condition variable with timeout and cancellation, and the bounded writer acquisition (`Db::lock_bounded` in `crates/tiramemsu/src/db.rs`) polls; neither records how long callers waited. The writer term dictionary (`TermDict`, `crates/tm-core/src/term.rs`) and the reader LRU (`TermReader`) do not count hits.

## Goals / Non-Goals

**Goals:**
- Fix the scale ceiling as a number the project is accountable for, and measure every core path at it.
- Turn benchmark results into a regression gate that runs without a human, and is cheap enough to run nightly.
- Find the cause of the as-of and current-view slowdowns at 1 000 versions before choosing a fix.
- Give operators a counters snapshot that costs nothing when nobody reads it.

**Non-Goals:**
- Changing the storage format, or committing to cold history now.
- Gating on absolute latencies across heterogeneous CI machines.
- A metrics push pipeline or tracing integration.

## Decisions

### D1. Ceiling: 10⁷ statements, 1 000 versions per hot key, 100 writes/s

These are the roadmap's proposal and match design D1 (10⁴–10⁷ statements). 100 writes/s is single-statement `assert` transactions with `synchronous = NORMAL` in WAL mode, sustained for 60 s on a file at 10⁷ statements. The ceiling is a promise about tested behavior, not a hard limit: nothing rejects an 11-millionth statement.

*Alternatives:* 10⁸ statements (would need the cold-history work first and a server-class fixture); leaving the ceiling open until a consumer asks (keeps H1 open, and leaves LFTJ routing thresholds unanchored).

### D2. A JSON-emitting harness for scale, criterion for micro-benchmarks

Criterion's statistical sampling is too slow to build and sample at 10⁷ nightly, and its output is not a stable machine format. The scale harness is a release-mode binary that builds each fixture once (through `ImportSession`, so fixture build time is itself the bulk-import measurement), runs each shape for a fixed number of iterations after warm-up, and writes `{metric, unit, n, p50, p95, p99, rows, plan}` per measurement. Every measurement first runs its query once and compares the row count with the fixture's known answer; a mismatch fails the run instead of recording a time.

*Alternatives:* criterion with `--save-baseline` and `critcmp` (good for local A/B, but no row checks and slow at 10⁷); `iai`/cachegrind instruction counts (stable on CI, but blind to I/O and page-cache effects, which are exactly the suspected cause).

### D3. Gate on ratios and on the same machine's baseline, with a margin

Absolute numbers vary across CI runners. The gate uses two kinds of metric:
- **Ratio metrics** (machine-independent): as-of throughput at N versions over the no-history baseline (target ≥ 0.70, the CozoDB bar already in the roadmap), current-view throughput at 1 000 versions over the no-history baseline (target ≥ 0.90), facade over direct `rusqlite` (recorded, not gated).
- **Baseline metrics:** p50 of each shape, compared with `bench/scale/baseline.json` recorded on the same runner class. A metric fails when it is more than 20 % slower than the baseline in the median of three runs. The baseline is updated only by an explicit commit.

Sustained writes are gated absolutely (≥ 100 commits/s with p99 commit latency ≤ 50 ms), since that is the ceiling's own definition. The ratio targets start as *recorded* until the history-depth fix lands; they become gating in the same change once met, or the change records why a target was revised.

*Alternatives:* a fixed self-hosted runner (more stable, more ops); gating on criterion's change detection (noisy at small sizes).

### D4. Diagnose before fixing history depth

The harness captures, per shape and view, `EXPLAIN QUERY PLAN`, the SQLite page-cache hit and miss counters (`SQLITE_DBSTATUS_CACHE_HIT/MISS` through the host, where available) and rows examined. The candidate causes are tested in order of cost: the covering `hist_*` index not chosen (or chosen with a poor prefix) for as-of; the instant-based `as_of` subquery repeated per pattern (L3: resolve the instant to a `t` once per read); page-cache pressure from history rows interleaved with live rows; and only then a layout change. Fixes that change no format land here; a layout change is written up as a follow-up change with the measurements attached.

*Alternatives:* building the cold-history table now (format bump, unproven benefit); partial `hist_*` indexes (the size benchmark was meant to decide this and is part of the harness).

### D5. Metrics are counters kept since open, read as a snapshot

`DbMetrics` is a plain value. Counters are `u64` updated where the engine already holds the relevant lock (writer guard, pool mutex, dictionary borrow), or relaxed atomics where it does not, so reading the snapshot takes no writer lock and runs no SQL except two cheap `PRAGMA`s for the storage section (`page_count`, `freelist_count`). Durations are measured with the facade's monotonic clock and reported in microseconds. The snapshot has a `layout` version field; new fields may be added in later minor versions, none removed before 1.0.

Fields (names final in task 1): `opened_at`, `uptime_us`; `writer { commits, dry_runs, lock_waits, lock_wait_us_total, lock_wait_us_max }`; `maintenance { optimize_runs, optimize_us_total, analyze_runs, analyze_us_total, text_catch_up_runs, text_catch_up_us_total, statistics_due }`; `readers { size, acquisitions, waits, wait_us_total, wait_us_max, timeouts }`; `term_cache { writer: { hits, misses, clears, len, capacity }, readers: { hits, misses, evictions, len, capacity } }`; `storage { format_version, last_t, page_size, page_count, freelist_count }`.

*Alternatives:* a `metrics` SQL view (would put mutable process state into the file's namespace); a callback/observer API (more flexible, but every binding needs a callback bridge); counting statements with `COUNT(*)` (a full scan at 10⁷, so excluded).

### D6. The MCP tool is read-only and always listed

`metrics` takes no arguments (`additionalProperties: false`), carries `readOnlyHint: true`, and is listed in read-only mode. It returns the snapshot JSON and nothing about data content, so it reveals no memory to an agent that could not already query it.

## Risks / Trade-offs

- [CI runners are noisy and the gate flakes] → Median of three runs, 20 % margin, ratio metrics where possible, and a manual re-run before a red gate blocks a release.
- [The 10⁷ fixture takes long to build] → Built weekly, cached as a workflow artifact keyed by the fixture generator's version; the nightly gate uses 10⁶.
- [History-depth targets may not be reachable without a format change] → The change records the measured cause and a revised target or a follow-up proposal; it does not silently lower the bar.
- [Counters add overhead to hot paths] → Increments only, no allocation; the facade-overhead benchmark is re-run with metrics on to show the difference is within noise.
- [oxilite numbers may favour oxilite] → Published either way, as the roadmap commits.

## Migration Plan

No data or API migration. `Db::metrics()` and the bridge op are additive. The nightly workflow starts in report-only mode for two weeks to settle the baseline, then becomes gating; the release check is enabled once the gate is green.

## Open Questions

- Should the 10⁷ weekly run also be gating, or remain evidence only? (Proposed: evidence only until a self-hosted runner exists.)
- Is p99 ≤ 50 ms the right commit-latency bound for 100 writes/s on laptop-class storage, or should it be stated per filesystem?
- Should reader-pool wait time be bucketed (a small histogram) rather than total and max?
