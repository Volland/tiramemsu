# Benchmarks

`cargo bench -p tiramemsu --bench <core|sparql|path|hotspots|vector>`. No targets are asserted; the numbers below
are recorded to steer tuning (`lat.md/roadmap#Benchmarks`).

The `facade_overhead` group compares full `Triple` decoding with a read transaction per lookup on both sides. It includes facade locking, pooling, and executor work; it does not isolate dynamic dispatch. The direct and facade result vectors are checked for equality before timing. SPARQL measurements include text parsing and planning.

## vector (add-vector-recall)

`VECTOR_BENCH_STATEMENTS` embedded statements (pseudo-random vectors, cosine), a now view, top 10. Same machine as above, `--warm-up-time 1 --measurement-time 3` (5 at 100 000). The numbers measure the scan, not retrieval quality: on random vectors the prefilter's recall is not meaningful, so measure it on real embeddings before relying on it.

| Statements | Dims, encoding | Exact | Prefilter 200 |
|---|---|---|---|
| 10 000 | 384 f32 | 7.4 ms | 2.4 ms |
| 10 000 | 768 f32 | 11.6 ms | 2.5 ms |
| 10 000 | 768 f16 | 11.0 ms | 2.5 ms |
| 100 000 | 384 f32 | 84.8 ms | 31.0 ms |
| 100 000 | 768 f32 | 134.4 ms | 31.3 ms |
| 100 000 | 768 f16 | 116.2 ms | 30.9 ms |

The first implementation took 70.9 ms and 877 ms for 768 f32 at 10 000 and 100 000. Three changes brought it down:
- The scan now reads only `(eid, blob)` with no `ORDER BY` and no per-row statement details or instant subquery; those are read for the hits only.
- Vectors decode into one reused buffer, and f16 through a 65 536-entry table.
- The covering index `embedding_bits` lets the prefilter read sign bits from the index instead of one page per row.

Cost now grows linearly at about 1.2 µs per 768-dimension embedding, exact.

**Prefilter quality on real vectors** (GloVe-100 angular from ann-benchmarks; first 100 000 / 200 000 vectors, 200 test queries; release build, Apple M2 Max; recall@10 against the engine's exact top 10, which matched numpy brute force on 20 of 20 sampled queries):

| n | 100k recall | 100k ms | 200k recall | 200k ms |
|---|---|---|---|---|
| exact | 1.000 | 44.5 | 1.000 | 89.9 |
| 200 | 0.544 | 28.5 | 0.515 | 58.1 |
| 1 000 | 0.785 | 30.8 | 0.754 | 60.6 |
| 5 000 | 0.945 | 46.0 | 0.913 | 72.5 |
| 10 000 | 0.979 | 92.2 | 0.961 | 96.3 |

With only 100 sign bits the prefilter needs about 5 % of the corpus as candidates for 0.95 recall, where it is no faster than exact search, and its fixed scan cost is about two thirds of exact. Treat it as a coarse trade-off; GloVe is a hard, low-dimensional set, so 384/768-d sentence embeddings are expected to fare better (not yet measured).

**Hybrid recall** (`recall_<n>_384`: notes with text, text index on, 384-d vectors; `topic7` matches 2 % of notes):

| Statements | Text + vector | With `expand: 2` | Text only |
|---|---|---|---|
| 10 000 | 10.0 ms | 15.2 ms | 0.49 ms |
| 100 000 | 90.7 ms | 112.4 ms | 7.6 ms |

The vector channel dominates. Expansion reads each distinct subject once per call (all notes here share one subject, so one sort of its statements). An approximate tier is the next step past roughly 10⁶ vectors (`lat.md/query#Vector Recall#Future Tiers`).

## 0.4.1 review fixes (2026-10-10)

Before (`5a55efc`, 0.4.0) and after the fixes of `docs/project-review.md`, same machine (Apple laptop, macOS arm64, Rust 1.91.1, bundled SQLite 3.53), run one after the other, `--warm-up-time 1 --measurement-time 3`. Criterion central estimates.

| Benchmark | Before | After | What changed |
|---|---|---|---|
| `sparql_latency_100000/point/now` | 55.4 µs | 20.2 µs | term lookups no longer re-prepare under STAT4 |
| `sparql_latency_100000/two_hop/now` | 92.1 µs | 45.4 µs | same |
| `sparql_latency_100000/point/asOf` | 59.7 µs | 24.8 µs | same |
| `sparql_latency_100000/two_hop/asOf` | 120.2 µs | 69.1 µs | same |
| `sparql_latency_100000/point/validAt` | 66.7 µs | 27.9 µs | same |
| `sparql_latency_100000/two_hop/validAt` | 120.9 µs | 73.6 µs | same |
| `churn/now/1000` | 16.6 µs | 2.7 µs | `View::triples` no longer re-prepares (now equal to `churn/now/1`) |
| `churn/as_of/1000` | 77.1 µs | 66.2 µs | still scans every version of the key: the open part of H1 |
| `supersede/cascade_set/1000` | 83.3 ms | 67.8 ms | batched cascade walk and row load |
| `supersede/cascade_set/100` | 7.88 ms | 7.35 ms | same |
| `hotspots/term_cache/warm_after_overflow/1000` | 6.53 ms | 6.35 ms | no measurable change on this fixture: the hot transaction is dominated by existence checks, not term lookups |
| `hotspots/bounded_writer/two_threads/400` | 83.8 ms | 84.5 ms | no throughput change; the fix removes polling, not contention |
| `hotspots/as_of_instant/two_hop` (tx vs instant) | 875 / 870 µs | 855 / 854 µs | instant resolution costs nothing measurable, so review finding L3 needed no change |

Root cause of the current-view churn regression of the first review: with STAT4 samples SQLite re-prepares a statement whenever a bare `?N` compared with an indexed column is bound. On a raw connection the same point lookup took 16.1 µs with STAT4 and 1.6 µs without; `col = +?N` keeps the covering-index seek and takes 1.9 µs. `tm-core`'s single-table lookups now use it; generated BGP joins keep bare parameters, because value-based planning is what orders them well (`lat.md/query.md#Join Ordering`).

## path (add-path-engine, M3)

`PATH_BENCH_STATEMENTS` sets the power-law graph (default 100 000 statements, 5 statements per
node, hub-skewed targets; `1000000` is the roadmap size and `10000000` the large one). Run on an
Apple laptop, bundled SQLite, `--warm-up-time 1 --measurement-time 2`, 100 000 statements.

| Group | Shape | Now | AsOf | ValidAt |
|---|---|---|---|---|
| `shortest` | `link+` ANY_SHORTEST to all endpoints from a random start, at most 6 hops | 5.9 ms | 6.5 ms | not run |
| `trail3` | `link{1,3}` TRAIL from a random start | 118 µs | 145 µs | 144 µs |
| `trail3` | `sys:anyRelationship{1,3}` TRAIL | 95 µs | 134 µs | 95 µs |
| `reach` | `next+` REACH over a 100 000-hop chain | 1.38 s | | |
| `reach` | `near+` REACH over a 50 000-node small world | 38 ms | | |
| `reach` | `step+` ALL_SHORTEST over a 30 x 30 grid (12 hops) | 10 ms | | |

A long chain is the worst case: every hop is one layer and so one statement (about 14 µs per
layer). Wide frontiers amortise the statement cost over up to a whole chunk.

The shortest benchmark is named `any_shortest_all_endpoints`; older runs labeled it `any_shortest_pairs` but filtered destinations only after the full search. Trail failures now abort a run rather than count as zero-result samples. The table above records the older run and is not a fresh measurement of the patch.

### `rarray` chunk sizes

`rarray_chunk/reach` runs `link+` (4 hops) from one start over the 100 000-statement graph through
`PathEngine` with the frontier chunk size set to 64, 256 and 1024.

| Chunk | Time |
|---|---|
| 64 | 465 µs |
| 256 | 306 µs |
| 1024 | 302 µs |

256 and 1024 are equal within noise and 64 is 50 % slower, so the default stays 256
(`crates/tm-exec/src/path/fetch.rs#DEFAULT_BATCH`).
