# Backend benchmark: SQLite, redb and Oxigraph

tiramemsu on SQLite (with and without the LFTJ operator), tiramemsu on redb, and [Oxigraph](https://github.com/oxigraph/oxigraph) 0.5 on RocksDB. All four run the same generated data and the same queries. This is the evidence behind decisions D42 and D43 in `lat.md/backends.md`. The numbers below are from the run after the `tune-native-runner` change (2026-10-27); `article/redb-backend/` quotes the 0.8.0 run before it.

## Our position before the numbers

tiramemsu prefers **transactional guarantees and security over raw speed**, and these numbers should be read in that light:

- **Transactions:** every tiramemsu write is one ACID transaction. It allocates a statement id and a transaction number, checks predicate schemas and cardinality, records an event, keeps the full history and cascades retractions to the statements built on top. An Oxigraph insert adds a quad. The write columns compare different amounts of work.
- **Durability:** both tiramemsu engines run at `Normal` (SQLite `synchronous = NORMAL` with WAL; redb group commit within 50 ms or 1 000 commits). Oxigraph runs at its default, RocksDB with its write-ahead log.
- **Security:** no engine opens a network port. The redb engine has no unsafe code and no C. Every failure is a typed error, and every query can be bounded by a budget.

## How it runs

```sh
cargo run --release -- run --scale 100000 --reps 5 --out results-100000.json
cargo run --release -- report results-100000.json
tm bench --scale 100000 --reps 5          # SQLite and redb only, no RocksDB build
python3 build-site.py                      # regenerate site/benchmarks.html from both results files
```

This crate is its own workspace, because Oxigraph builds RocksDB (C++). The workloads, generators and checks live in `crates/tm-bench`.

- **Data:** a seeded WatDiv-style e-commerce graph (users, products, retailers and reviews, with skewed follows, likes and review targets) and the triangle benchmark's uniform and hub-and-spoke graphs. Every engine gets the same triples through its bulk path.
- **Correctness before timing:** every query runs once on every engine and the results are compared, counts always and sorted rows up to 1 000 rows. A mismatch stops the run. A query that passes **60 s** on an engine is recorded as a timeout and left out of that query's comparison.
- **Timing:** one checked run, then five timed runs. Medians and p95 are reported, throughput for concurrent readers, bytes per statement for size.
- **What each engine can answer:** Oxigraph has no transaction or valid time, no statement ids and no Cypher, so it takes no part in the as-of and valid-at lookups, Cypher, Cypher paths, cascade and supersede. Its "replace" is a remove and an insert in one transaction.

## Strengths and weaknesses

At 10⁵ statements, after the `tune-native-runner` change (zero-copy value reads, flat join rows, a join order tried from every first pattern, and constant equalities as seeks; see `openspec/changes/tune-native-runner/`):

**redb, compared with SQLite**

- Stronger:
  - cyclic patterns, now on every graph: a skewed triangle 57× faster than SQLite's own plan and 1.5× faster than SQLite with LFTJ; the uniform 4-cycle 4.8× and diamond 3.8× faster;
  - the Cypher 2-hop with a property map (6×: 0.32 ms against 1.9 ms), because `{name: 'User 7'}` is now a seek;
  - concurrent readers (3.1× at 16 threads, 2.1× at 4);
  - bulk load (2.5×);
  - the snowflake F1 (2.1×: 6.7 ms against 13.9 ms) and the Cypher star (1.6×);
  - lookups in every view (1.2–1.6×).
- Weaker:
  - Cypher variable-length paths (shortest path and trail, about 2.5× slower); the path engine was not part of this change;
  - stars and aggregates with selective constants (S1, S2, S3, C1, C2, F2: 1.4–1.8× slower, down from 2.1–3.8×);
  - file size (3.5× larger: six live and three history permutations of fixed-width keys);
  - replace (1.4×) and cascade (1.1×) writes.

**What the change moved** (redb at 10⁵, against the 0.8.0 run): C1 2.5×, F1 2.0×, S3 1.9×, C2 1.8×, S1 1.6× faster; the Cypher 2-hop 16.5×; every cyclic query about 2×; lookups 1.2–1.3×. Load, size and writes are unchanged. Concurrent readers moved −17 % at 4 threads and −9 % at 16 here but +20 % and +1 % at 10⁶; the read path these threads share did not change, so we read both as run-to-run noise. A larger redb page cache (2 GiB instead of 256 MiB) changed no query time.

**SQLite with LFTJ** is close to redb on skewed cyclic patterns (7.0 ms against 4.7 ms for a triangle) and between SQLite's plan and redb on uniform ones. Without LFTJ, SQLite's nested loops are 33–38× slower on skewed triangles than with it.

**Oxigraph**

- Stronger: writes (an insert takes 5 µs against about 120 µs), bulk load (19–47×), size (5–17× smaller) and 4- and 16-thread reader throughput at 10⁵ (1.2× and 1.8× redb).
- Weaker: every join workload. WatDiv templates are 2–31× slower than redb and cyclic patterns 15–740× slower (at 10⁶: 2–51× and 13–2 700×).

**All engines** take seconds on the dense diamonds and 4-cycles of the skewed graph, because those queries have millions of matches. A worst-case-optimal join bounds the wasted work, not the size of the output.

**At 10⁶ statements** the picture holds, with these changes:

- **Cyclic patterns:** a skewed triangle takes 21 ms on redb, 29 ms on SQLite with LFTJ, 4.9 s on SQLite's own plan and 55 s on Oxigraph. redb is now the only engine that answers the skewed diamond and 4-cycle within the 60 s cap on its checked run (4-cycle 38 s). Its five timed diamond runs took 69 s each (the cap bounds the checked run only), so the derived list below counts it as "at least 1.2× slower" than a SQLite that never finished. No other engine answered either query, so their results are not cross-checked in this run; the differential tests check the same queries on small graphs.
- **Joins:** the snowflake F1 is no longer a loss: 20 ms on redb against 23 ms on SQLite (0.8.0: 83 ms, 3.6× slower). The order that started from the smallest pattern grew 315 000 intermediate rows; trying every first pattern keeps it near 10 000. S3 is 1.7×, C1 1.5×, S1 1.3×, L1 1.2× and C2 1.1× slower; S2 and F2 are even.
- **Cypher:** the 2-hop takes 0.38 ms against 14 ms on SQLite (38×); the star is 1.6× faster and the aggregate even.
- **Readers:** redb is the fastest engine on one thread (18 700 lookups per second, against 14 700 on SQLite and 11 700 on Oxigraph) and on four (36 700 against 34 300 on Oxigraph); Oxigraph stays ahead on 16 threads (67 600 against 49 500).
- **Size:** a fresh bulk load leaves redb at 2 147 bytes per statement, 7.5× SQLite, because redb grows its file in large steps while loading. `tm compact` brings it to 1 051 (3.7× SQLite), so compact after a bulk load.
- **Load:** redb loads 1.7× faster than SQLite (51 s against 89 s for a million statements); Oxigraph takes 2.0 s.
- **Writes:** cascade and supersede are about 40 % slower on redb than on SQLite at this size; insert and replace are within 10 %.
- **Lookups:** point and two-hop lookups stay at 43–73 µs on both tiramemsu engines in every view, redb the faster in each.

## Results at 10⁶ statements

Generated from `results-1000000.json` by `cargo run --release -- report results-1000000.json`.

### Run

- **Machine:** macos aarch64 · Apple M2 Max · 12 threads
- **Scale:** 1004143 statements in the WatDiv-style graph, 5 timed repetitions per query after one checked run
- **Seeds:** skewed 7, watdiv 7
- **Versions:** oxigraph: Oxigraph 0.5 on RocksDB; redb: tiramemsu 0.8.0 on redb; sqlite: tiramemsu 0.8.0 on SQLite 3.53.2; sqlite-lftj: tiramemsu 0.8.0 on SQLite 3.53.2
- **Durability:** tiramemsu: Normal on both backends; Oxigraph: its default (RocksDB WAL)
- **Time cap:** 60 s per query; an engine that passes it is shown as a timeout and left out of that query's comparison
- **Command:** `./target/release/tm-bench-backends run --scale 1000000 --reps 5 --out results-1000000.json`

#### load

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| bulk load | watdiv | - | 89.07 s | 51.09 s | 2.00 s |

#### size

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| after load | watdiv | - | 287 B | 2147 B | 78 B |
| after churn | watdiv | - | 288 B | 2147 B | 130 B |
| after compaction | watdiv | - | - | 1051 B | 130 B |

#### watdiv

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| L1 | now | 7320 | 4.01 ms (p95 4.36 ms) | 4.77 ms (p95 5.10 ms) | 28.57 ms (p95 30.94 ms) |
| L2 | now | 0 | 63 µs (p95 118 µs) | 41 µs (p95 65 µs) | 72 µs (p95 84 µs) |
| L3 | now | 9 | 69 µs (p95 101 µs) | 58 µs (p95 76 µs) | 162 µs (p95 183 µs) |
| S1 | now | 1654 | 5.67 ms (p95 6.31 ms) | 7.63 ms (p95 8.43 ms) | 37.04 ms (p95 38.11 ms) |
| S2 | now | 1138 | 2.69 ms (p95 3.46 ms) | 2.70 ms (p95 3.50 ms) | 12.62 ms (p95 14.01 ms) |
| S3 | now | 598 | 2.23 ms (p95 3.86 ms) | 3.84 ms (p95 5.18 ms) | 195.98 ms (p95 197.15 ms) |
| F1 | now | 10581 | 23.17 ms (p95 23.98 ms) | 20.22 ms (p95 20.83 ms) | 179.34 ms (p95 194.63 ms) |
| F2 | now | 1935 | 14.48 ms (p95 18.98 ms) | 14.29 ms (p95 15.40 ms) | 74.26 ms (p95 78.79 ms) |
| C1 | now | 19 | 13.41 ms (p95 14.14 ms) | 20.72 ms (p95 22.13 ms) | 400.65 ms (p95 454.37 ms) |
| C2 | now | 1 | 87.07 ms (p95 94.38 ms) | 96.17 ms (p95 97.59 ms) | 602.75 ms (p95 669.97 ms) |

#### lookup

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| point | now | 10 | 59 µs (p95 121 µs) | 47 µs (p95 74 µs) | 115 µs (p95 120 µs) |
| point | asOf | 10 | 70 µs (p95 75 µs) | 43 µs (p95 46 µs) | - |
| point | validAt | 10 | 69 µs (p95 73 µs) | 52 µs (p95 109 µs) | - |
| 2-hop | now | 9 | 52 µs (p95 110 µs) | 48 µs (p95 88 µs) | 140 µs (p95 158 µs) |
| 2-hop | asOf | 9 | 73 µs (p95 96 µs) | 46 µs (p95 54 µs) | - |
| 2-hop | validAt | 9 | 68 µs (p95 80 µs) | 45 µs (p95 49 µs) | - |

#### cypher

| query | variant | rows | sqlite | redb |
|---|---|---|---|---|
| 2-hop | now | 9 | 14.44 ms (p95 15.49 ms) | 379 µs (p95 567 µs) |
| star | now | 827 | 152.47 ms (p95 156.28 ms) | 95.12 ms (p95 100.30 ms) |
| aggregate | now | 19 | 389.70 ms (p95 404.14 ms) | 372.44 ms (p95 386.84 ms) |

#### paths

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| reachability | now | 1 | 34.15 ms (p95 36.03 ms) | 33.33 ms (p95 33.93 ms) | 107.21 ms (p95 125.50 ms) |
| shortest | now | 1 | 1.61 s (p95 1.63 s) | 3.87 s (p95 4.12 s) | - |
| trail 1..3 | now | 1 | 760.08 ms (p95 762.78 ms) | 1.92 s (p95 1.94 s) | - |

#### concurrency

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| 2-hop | 1 threads | - | 14724 q/s | 18680 q/s | 11700 q/s |
| 2-hop | 4 threads | - | 13934 q/s | 36746 q/s | 34344 q/s |
| 2-hop | 16 threads | - | 15897 q/s | 49496 q/s | 67577 q/s |

#### writes

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| insert | watdiv | - | 141 µs (p95 447 µs) | 146 µs (p95 213 µs) | 6 µs (p95 23 µs) |
| replace (churn) | watdiv | - | 187 µs (p95 319 µs) | 203 µs (p95 256 µs) | 16 µs (p95 20 µs) |
| cascade | watdiv | - | 384 µs (p95 1.94 ms) | 538 µs (p95 743 µs) | - |
| supersede | watdiv | - | 253 µs (p95 713 µs) | 345 µs (p95 453 µs) | - |

#### cyclic

| query | variant | rows | sqlite | sqlite-lftj | redb | oxigraph |
|---|---|---|---|---|---|---|
| triangle | uniform | 1 | 923.85 ms (p95 943.15 ms) | 550.99 ms (p95 560.72 ms) | 492.07 ms (p95 528.05 ms) | 7.58 s (p95 7.60 s) |
| diamond | uniform | 1 | 3.58 s (p95 3.61 s) | 2.24 s (p95 2.27 s) | 1.24 s (p95 1.28 s) | 36.98 s (p95 37.70 s) |
| 4-cycle | uniform | 1 | 3.57 s (p95 3.62 s) | 1.30 s (p95 1.45 s) | 1.14 s (p95 1.32 s) | 38.81 s (p95 39.05 s) |
| tailed triangle | uniform | 1 | 882.28 ms (p95 903.63 ms) | 682.66 ms (p95 700.31 ms) | 556.29 ms (p95 625.90 ms) | 7.17 s (p95 7.20 s) |
| triangle | skewed | 1 | 4.87 s (p95 4.90 s) | 29.00 ms (p95 29.37 ms) | 20.76 ms (p95 20.86 ms) | 55.35 s (p95 55.92 s) |
| diamond | skewed | - | timeout (> 60 s) | timeout (> 60 s) | 69.26 s (p95 70.68 s) | timeout (> 60 s) |
| 4-cycle | skewed | - | timeout (> 60 s) | timeout (> 60 s) | 37.62 s (p95 38.21 s) | timeout (> 60 s) |
| tailed triangle | skewed | 1 | 5.52 s (p95 5.58 s) | 35.90 ms (p95 36.82 ms) | 27.97 ms (p95 30.26 ms) | 55.79 s (p95 57.37 s) |

### Strengths and weaknesses (derived)

Each line compares one measurement and names the factor; differences under 10 % are left out.

**redb faster or smaller than sqlite** (21):

- cyclic / triangle (skewed): 234.7×
- cyclic / tailed triangle (skewed): 197.4×
- cypher / 2-hop (now): 38.1×
- cyclic / 4-cycle (uniform): 3.1×
- concurrency / 2-hop (16 threads): 3.1×
- cyclic / diamond (uniform): 2.9×
- concurrency / 2-hop (4 threads): 2.6×
- cyclic / triangle (uniform): 1.9×
- load / bulk load (watdiv): 1.7×
- lookup / point (asOf): 1.6×
- cypher / star (now): 1.6×
- lookup / 2-hop (asOf): 1.6×
- cyclic / 4-cycle (skewed) (at least: one side timed out): 1.6×
- cyclic / tailed triangle (uniform): 1.6×
- watdiv / L2 (now): 1.5×
- lookup / 2-hop (validAt): 1.5×
- lookup / point (validAt): 1.3×
- lookup / point (now): 1.3×
- concurrency / 2-hop (1 threads): 1.3×
- watdiv / L3 (now): 1.2×
- watdiv / F1 (now): 1.1×

**redb slower or larger than sqlite** (12):

- size / after load (watdiv): 7.5×
- size / after churn (watdiv): 7.5×
- paths / trail 1..3 (now): 2.5×
- paths / shortest (now): 2.4×
- watdiv / S3 (now): 1.7×
- watdiv / C1 (now): 1.5×
- writes / cascade (watdiv): 1.4×
- writes / supersede (watdiv): 1.4×
- watdiv / S1 (now): 1.3×
- watdiv / L1 (now): 1.2×
- cyclic / diamond (skewed) (at least: one side timed out): 1.2×
- watdiv / C2 (now): 1.1×

**redb faster or smaller than oxigraph** (21):

- cyclic / triangle (skewed): 2666.0×
- cyclic / tailed triangle (skewed): 1994.6×
- watdiv / S3 (now): 51.0×
- cyclic / 4-cycle (uniform): 33.9×
- cyclic / diamond (uniform): 29.9×
- watdiv / C1 (now): 19.3×
- cyclic / triangle (uniform): 15.4×
- cyclic / tailed triangle (uniform): 12.9×
- watdiv / F1 (now): 8.9×
- watdiv / C2 (now): 6.3×
- watdiv / L1 (now): 6.0×
- watdiv / F2 (now): 5.2×
- watdiv / S1 (now): 4.9×
- watdiv / S2 (now): 4.7×
- paths / reachability (now): 3.2×
- lookup / 2-hop (now): 2.9×
- watdiv / L3 (now): 2.8×
- lookup / point (now): 2.5×
- watdiv / L2 (now): 1.8×
- concurrency / 2-hop (1 threads): 1.6×
- cyclic / 4-cycle (skewed) (at least: one side timed out): 1.6×

**redb slower or larger than oxigraph** (8):

- size / after load (watdiv): 27.6×
- load / bulk load (watdiv): 25.6×
- writes / insert (watdiv): 24.7×
- size / after churn (watdiv): 16.5×
- writes / replace (churn) (watdiv): 13.0×
- size / after compaction (watdiv): 8.1×
- concurrency / 2-hop (16 threads): 1.4×
- cyclic / diamond (skewed) (at least: one side timed out): 1.2×

**sqlite faster or smaller than oxigraph** (20):

- watdiv / S3 (now): 88.0×
- watdiv / C1 (now): 29.9×
- cyclic / triangle (skewed): 11.4×
- cyclic / 4-cycle (uniform): 10.9×
- cyclic / diamond (uniform): 10.3×
- cyclic / tailed triangle (skewed): 10.1×
- cyclic / triangle (uniform): 8.2×
- cyclic / tailed triangle (uniform): 8.1×
- watdiv / F1 (now): 7.7×
- watdiv / L1 (now): 7.1×
- watdiv / C2 (now): 6.9×
- watdiv / S1 (now): 6.5×
- watdiv / F2 (now): 5.1×
- watdiv / S2 (now): 4.7×
- paths / reachability (now): 3.1×
- lookup / 2-hop (now): 2.7×
- watdiv / L3 (now): 2.3×
- lookup / point (now): 1.9×
- concurrency / 2-hop (1 threads): 1.3×
- watdiv / L2 (now): 1.1×

**sqlite slower or larger than oxigraph** (7):

- load / bulk load (watdiv): 44.6×
- writes / insert (watdiv): 23.8×
- writes / replace (churn) (watdiv): 12.0×
- concurrency / 2-hop (16 threads): 4.3×
- size / after load (watdiv): 3.7×
- concurrency / 2-hop (4 threads): 2.5×
- size / after churn (watdiv): 2.2×

## Results at 10⁵ statements

Generated from `results-100000.json`.

### Run

- **Machine:** macos aarch64 · Apple M2 Max · 12 threads
- **Scale:** 100352 statements in the WatDiv-style graph, 5 timed repetitions per query after one checked run
- **Seeds:** skewed 7, watdiv 7
- **Versions:** oxigraph: Oxigraph 0.5 on RocksDB; redb: tiramemsu 0.8.0 on redb; sqlite: tiramemsu 0.8.0 on SQLite 3.53.2; sqlite-lftj: tiramemsu 0.8.0 on SQLite 3.53.2
- **Durability:** tiramemsu: Normal on both backends; Oxigraph: its default (RocksDB WAL)
- **Time cap:** 60 s per query; an engine that passes it is shown as a timeout and left out of that query's comparison
- **Command:** `./target/release/tm-bench-backends run --scale 100000 --reps 5 --out results-100000.json`

#### load

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| bulk load | watdiv | - | 7.10 s | 2.85 s | 0.15 s |

#### size

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| after load | watdiv | - | 384 B | 1343 B | 79 B |
| after churn | watdiv | - | 388 B | 1343 B | 82 B |
| after compaction | watdiv | - | - | 1056 B | 135 B |

#### watdiv

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| L1 | now | 634 | 402 µs (p95 465 µs) | 403 µs (p95 441 µs) | 1.63 ms (p95 1.75 ms) |
| L2 | now | 121 | 166 µs (p95 201 µs) | 148 µs (p95 170 µs) | 481 µs (p95 523 µs) |
| L3 | now | 9 | 79 µs (p95 105 µs) | 65 µs (p95 78 µs) | 140 µs (p95 156 µs) |
| S1 | now | 136 | 503 µs (p95 643 µs) | 769 µs (p95 912 µs) | 2.19 ms (p95 2.87 ms) |
| S2 | now | 310 | 490 µs (p95 720 µs) | 687 µs (p95 827 µs) | 2.19 ms (p95 2.91 ms) |
| S3 | now | 51 | 225 µs (p95 312 µs) | 372 µs (p95 465 µs) | 11.34 ms (p95 11.42 ms) |
| F1 | now | 3874 | 13.90 ms (p95 14.33 ms) | 6.72 ms (p95 7.39 ms) | 42.15 ms (p95 43.35 ms) |
| F2 | now | 208 | 1.10 ms (p95 1.24 ms) | 1.50 ms (p95 1.75 ms) | 4.86 ms (p95 5.12 ms) |
| C1 | now | 20 | 1.16 ms (p95 1.24 ms) | 2.04 ms (p95 2.17 ms) | 28.87 ms (p95 29.30 ms) |
| C2 | now | 1 | 4.87 ms (p95 5.16 ms) | 7.33 ms (p95 7.56 ms) | 38.32 ms (p95 38.43 ms) |

#### lookup

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| point | now | 10 | 54 µs (p95 101 µs) | 39 µs (p95 62 µs) | 74 µs (p95 93 µs) |
| point | asOf | 10 | 58 µs (p95 68 µs) | 38 µs (p95 41 µs) | - |
| point | validAt | 10 | 57 µs (p95 63 µs) | 42 µs (p95 48 µs) | - |
| 2-hop | now | 8 | 45 µs (p95 55 µs) | 38 µs (p95 45 µs) | 85 µs (p95 119 µs) |
| 2-hop | asOf | 8 | 58 µs (p95 62 µs) | 37 µs (p95 40 µs) | - |
| 2-hop | validAt | 8 | 57 µs (p95 60 µs) | 37 µs (p95 39 µs) | - |

#### cypher

| query | variant | rows | sqlite | redb |
|---|---|---|---|---|
| 2-hop | now | 8 | 1.94 ms (p95 2.01 ms) | 323 µs (p95 433 µs) |
| star | now | 68 | 13.80 ms (p95 13.98 ms) | 8.68 ms (p95 9.22 ms) |
| aggregate | now | 20 | 23.82 ms (p95 26.62 ms) | 27.36 ms (p95 31.82 ms) |

#### paths

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| reachability | now | 1 | 2.74 ms (p95 2.83 ms) | 2.51 ms (p95 2.69 ms) | 5.63 ms (p95 7.08 ms) |
| shortest | now | 1 | 146.97 ms (p95 155.24 ms) | 364.77 ms (p95 401.99 ms) | - |
| trail 1..3 | now | 1 | 69.93 ms (p95 83.26 ms) | 183.83 ms (p95 212.61 ms) | - |

#### concurrency

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| 2-hop | 1 threads | - | 17568 q/s | 19668 q/s | 18414 q/s |
| 2-hop | 4 threads | - | 16686 q/s | 34883 q/s | 43311 q/s |
| 2-hop | 16 threads | - | 16109 q/s | 50397 q/s | 91192 q/s |

#### writes

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| insert | watdiv | - | 111 µs (p95 236 µs) | 120 µs (p95 150 µs) | 5 µs (p95 9 µs) |
| replace (churn) | watdiv | - | 119 µs (p95 197 µs) | 167 µs (p95 282 µs) | 17 µs (p95 108 µs) |
| cascade | watdiv | - | 409 µs (p95 1.97 ms) | 461 µs (p95 643 µs) | - |
| supersede | watdiv | - | 278 µs (p95 972 µs) | 291 µs (p95 432 µs) | - |

#### cyclic

| query | variant | rows | sqlite | sqlite-lftj | redb | oxigraph |
|---|---|---|---|---|---|---|
| triangle | uniform | 1 | 65.61 ms (p95 67.78 ms) | 45.00 ms (p95 45.29 ms) | 31.63 ms (p95 33.46 ms) | 683.14 ms (p95 724.99 ms) |
| diamond | uniform | 1 | 368.48 ms (p95 374.50 ms) | 185.54 ms (p95 186.16 ms) | 96.19 ms (p95 100.40 ms) | 3.36 s (p95 3.42 s) |
| 4-cycle | uniform | 1 | 366.68 ms (p95 376.57 ms) | 99.63 ms (p95 104.24 ms) | 76.95 ms (p95 78.39 ms) | 3.65 s (p95 3.95 s) |
| tailed triangle | uniform | 1 | 74.58 ms (p95 76.60 ms) | 57.71 ms (p95 61.01 ms) | 45.73 ms (p95 47.37 ms) | 695.19 ms (p95 706.01 ms) |
| triangle | skewed | 1 | 265.82 ms (p95 280.05 ms) | 7.04 ms (p95 7.23 ms) | 4.67 ms (p95 5.27 ms) | 3.44 s (p95 3.57 s) |
| diamond | skewed | 1 | 5.77 s (p95 5.83 s) | 7.13 s (p95 7.29 s) | 3.13 s (p95 3.46 s) | 48.89 s (p95 49.02 s) |
| 4-cycle | skewed | 1 | 4.30 s (p95 4.44 s) | 5.83 s (p95 5.91 s) | 1.41 s (p95 1.78 s) | 50.31 s (p95 52.19 s) |
| tailed triangle | skewed | 1 | 289.21 ms (p95 306.23 ms) | 8.66 ms (p95 8.78 ms) | 6.17 ms (p95 6.42 ms) | 3.47 s (p95 3.49 s) |

### Strengths and weaknesses (derived)

Each line compares one measurement and names the factor; differences under 10 % are left out.

**redb faster or smaller than sqlite** (23):

- cyclic / triangle (skewed): 56.9×
- cyclic / tailed triangle (skewed): 46.9×
- cypher / 2-hop (now): 6.0×
- cyclic / 4-cycle (uniform): 4.8×
- cyclic / diamond (uniform): 3.8×
- concurrency / 2-hop (16 threads): 3.1×
- cyclic / 4-cycle (skewed): 3.1×
- load / bulk load (watdiv): 2.5×
- concurrency / 2-hop (4 threads): 2.1×
- cyclic / triangle (uniform): 2.1×
- watdiv / F1 (now): 2.1×
- cyclic / diamond (skewed): 1.8×
- cyclic / tailed triangle (uniform): 1.6×
- cypher / star (now): 1.6×
- lookup / 2-hop (asOf): 1.6×
- lookup / point (asOf): 1.6×
- lookup / 2-hop (validAt): 1.5×
- lookup / point (now): 1.4×
- lookup / point (validAt): 1.4×
- watdiv / L3 (now): 1.2×
- lookup / 2-hop (now): 1.2×
- concurrency / 2-hop (1 threads): 1.1×
- watdiv / L2 (now): 1.1×

**redb slower or larger than sqlite** (13):

- size / after load (watdiv): 3.5×
- size / after churn (watdiv): 3.5×
- paths / trail 1..3 (now): 2.6×
- paths / shortest (now): 2.5×
- watdiv / C1 (now): 1.8×
- watdiv / S3 (now): 1.7×
- watdiv / S1 (now): 1.5×
- watdiv / C2 (now): 1.5×
- watdiv / S2 (now): 1.4×
- writes / replace (churn) (watdiv): 1.4×
- watdiv / F2 (now): 1.4×
- cypher / aggregate (now): 1.1×
- writes / cascade (watdiv): 1.1×

**redb faster or smaller than oxigraph** (21):

- cyclic / triangle (skewed): 736.5×
- cyclic / tailed triangle (skewed): 563.3×
- cyclic / 4-cycle (uniform): 47.4×
- cyclic / 4-cycle (skewed): 35.7×
- cyclic / diamond (uniform): 35.0×
- watdiv / S3 (now): 30.5×
- cyclic / triangle (uniform): 21.6×
- cyclic / diamond (skewed): 15.6×
- cyclic / tailed triangle (uniform): 15.2×
- watdiv / C1 (now): 14.1×
- watdiv / F1 (now): 6.3×
- watdiv / C2 (now): 5.2×
- watdiv / L1 (now): 4.0×
- watdiv / L2 (now): 3.2×
- watdiv / F2 (now): 3.2×
- watdiv / S2 (now): 3.2×
- watdiv / S1 (now): 2.9×
- lookup / 2-hop (now): 2.3×
- paths / reachability (now): 2.2×
- watdiv / L3 (now): 2.2×
- lookup / point (now): 1.9×

**redb slower or larger than oxigraph** (8):

- writes / insert (watdiv): 22.9×
- load / bulk load (watdiv): 19.0×
- size / after load (watdiv): 16.9×
- size / after churn (watdiv): 16.4×
- writes / replace (churn) (watdiv): 9.9×
- size / after compaction (watdiv): 7.8×
- concurrency / 2-hop (16 threads): 1.8×
- concurrency / 2-hop (4 threads): 1.2×

**sqlite faster or smaller than oxigraph** (21):

- watdiv / S3 (now): 50.4×
- watdiv / C1 (now): 24.9×
- cyclic / triangle (skewed): 12.9×
- cyclic / tailed triangle (skewed): 12.0×
- cyclic / 4-cycle (skewed): 11.7×
- cyclic / triangle (uniform): 10.4×
- cyclic / 4-cycle (uniform): 9.9×
- cyclic / tailed triangle (uniform): 9.3×
- cyclic / diamond (uniform): 9.1×
- cyclic / diamond (skewed): 8.5×
- watdiv / C2 (now): 7.9×
- watdiv / S2 (now): 4.5×
- watdiv / F2 (now): 4.4×
- watdiv / S1 (now): 4.4×
- watdiv / L1 (now): 4.1×
- watdiv / F1 (now): 3.0×
- watdiv / L2 (now): 2.9×
- paths / reachability (now): 2.1×
- lookup / 2-hop (now): 1.9×
- watdiv / L3 (now): 1.8×
- lookup / point (now): 1.4×

**sqlite slower or larger than oxigraph** (7):

- load / bulk load (watdiv): 47.3×
- writes / insert (watdiv): 21.3×
- writes / replace (churn) (watdiv): 7.1×
- concurrency / 2-hop (16 threads): 5.7×
- size / after load (watdiv): 4.8×
- size / after churn (watdiv): 4.7×
- concurrency / 2-hop (4 threads): 2.6×
