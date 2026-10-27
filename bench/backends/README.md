# Backend benchmark: SQLite, redb and Oxigraph

tiramemsu on SQLite (with and without the LFTJ operator), tiramemsu on redb, and [Oxigraph](https://github.com/oxigraph/oxigraph) 0.5 on RocksDB. All four run the same generated data and the same queries. This is the evidence behind decisions D42 and D43 in `lat.md/backends.md`, and the source of the numbers in `article/redb-backend/`.

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
```

This crate is its own workspace, because Oxigraph builds RocksDB (C++). The workloads, generators and checks live in `crates/tm-bench`.

- **Data:** a seeded WatDiv-style e-commerce graph (users, products, retailers and reviews, with skewed follows, likes and review targets) and the triangle benchmark's uniform and hub-and-spoke graphs. Every engine gets the same triples through its bulk path.
- **Correctness before timing:** every query runs once on every engine and the results are compared, counts always and sorted rows up to 1 000 rows. A mismatch stops the run. A query that passes **60 s** on an engine is recorded as a timeout and left out of that query's comparison.
- **Timing:** one checked run, then five timed runs. Medians and p95 are reported, throughput for concurrent readers, bytes per statement for size.
- **What each engine can answer:** Oxigraph has no transaction or valid time, no statement ids and no Cypher, so it takes no part in the as-of and valid-at lookups, Cypher, Cypher paths, cascade and supersede. Its "replace" is a remove and an insert in one transaction.

## Strengths and weaknesses

At 10⁵ statements, after the four native-runner fixes described in `article/redb-backend/wcoj-on-kv.md`:

**redb, compared with SQLite**

- Stronger:
  - cyclic patterns on skewed graphs (triangle 27× faster, tailed triangle 23×, against SQLite's own plan);
  - concurrent readers (3.3× at 16 threads, 2.4× at 4);
  - bulk load (2.5×);
  - lookups in time views (as-of and valid-at, about 1.4×);
  - chains and snowflakes (L1, L3 and F1 slightly faster).
- Weaker:
  - stars and aggregates with selective constants (S1, S3, C1, C2: 2.1–3.8× slower), because a redb B-tree seek costs more than a SQLite covering-index probe;
  - Cypher variable-length paths (shortest path and trail, about 3× slower);
  - the Cypher 2-hop with property projection (about 3×);
  - file size (3.5× larger: six live and three history permutations of fixed-width keys);
  - single writes (5–30 % slower).

**SQLite with LFTJ** is the fastest engine on uniform cyclic patterns and close to redb on skewed ones. Without LFTJ, SQLite's nested loops are 33–36× slower on skewed triangles than with it.

**Oxigraph**

- Stronger: writes (an insert takes 5 µs against about 120 µs), bulk load (18–46×), size (5–17× smaller) and 16-thread reader throughput (1.7× redb).
- Weaker: every join workload. WatDiv templates are 2–37× slower than tiramemsu and cyclic patterns 8–355× slower; skewed diamonds and 4-cycles take 48–50 s against 4–7 s.

**All engines** take seconds on the dense diamonds and 4-cycles of the skewed graph, because those queries have millions of matches. A worst-case-optimal join bounds the wasted work, not the size of the output.

**At 10⁶ statements** the picture holds, with these changes:

- **Cyclic patterns:** the gap widens. A skewed triangle takes 45 ms on redb against 5.1 s on SQLite's own plan (113×) and 57 s on Oxigraph. The dense diamonds and 4-cycles of the skewed graph pass the 60 s cap on every engine.
- **Readers:** redb is the fastest engine on one thread (15 800 lookups per second, against 12 500 on SQLite and 11 500 on Oxigraph) and 3.2× SQLite on 16 threads; Oxigraph stays ahead on 4 and 16 threads.
- **Joins:** the WatDiv snowflake F1 becomes a clear redb loss (83 ms against 23 ms, 3.6×). C1 stays 2.5× and S3 2.6× slower. The Cypher star and aggregate are now faster on redb than on SQLite.
- **Size:** a fresh bulk load leaves redb at 2 147 bytes per statement, 7.5× SQLite, because redb grows its file in large steps while loading. `tm compact` brings it to 1 051 (3.7× SQLite), so compact after a bulk load.
- **Load:** redb loads 1.7× faster than SQLite (55 s against 94 s for a million statements); Oxigraph takes 1.9 s.
- **Writes:** cascade and supersede are about 40 % slower on redb than on SQLite at this size; insert and replace are within 10 %.
- **Lookups:** point and two-hop lookups stay at 45–65 µs on both tiramemsu engines in every view, about twice as fast as Oxigraph's.

## Results at 10⁶ statements

Generated from `results-1000000.json` by `cargo run --release -- report results-1000000.json`.

### Run

- **Machine:** macos aarch64 · Apple M2 Max · 12 threads
- **Scale:** 1004143 statements in the WatDiv-style graph, 5 timed repetitions per query after one checked run
- **Seeds:** skewed 7, watdiv 7
- **Versions:** oxigraph: Oxigraph 0.5 on RocksDB; redb: tiramemsu 0.7.0 on redb; sqlite: tiramemsu 0.7.0 on SQLite 3.53.2; sqlite-lftj: tiramemsu 0.7.0 on SQLite 3.53.2
- **Durability:** tiramemsu: Normal on both backends; Oxigraph: its default (RocksDB WAL)
- **Time cap:** 60 s per query; an engine that passes it is shown as a timeout and left out of that query's comparison
- **Command:** `./target/release/tm-bench-backends run --scale 1000000 --reps 5 --out results-1000000.json`

#### load

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| bulk load | watdiv | - | 94.02 s | 54.94 s | 1.90 s |

#### size

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| after load | watdiv | - | 287 B | 2147 B | 78 B |
| after churn | watdiv | - | 288 B | 2147 B | 130 B |
| after compaction | watdiv | - | - | 1051 B | 130 B |

#### watdiv

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| L1 | now | 7320 | 4.06 ms (p95 4.26 ms) | 5.94 ms (p95 6.04 ms) | 29.13 ms (p95 30.96 ms) |
| L2 | now | 0 | 71 µs (p95 127 µs) | 42 µs (p95 70 µs) | 118 µs (p95 168 µs) |
| L3 | now | 9 | 85 µs (p95 138 µs) | 63 µs (p95 96 µs) | 165 µs (p95 196 µs) |
| S1 | now | 1654 | 6.01 ms (p95 6.18 ms) | 10.79 ms (p95 11.32 ms) | 36.11 ms (p95 38.46 ms) |
| S2 | now | 1138 | 2.86 ms (p95 3.87 ms) | 3.48 ms (p95 4.09 ms) | 12.40 ms (p95 13.49 ms) |
| S3 | now | 598 | 2.30 ms (p95 4.10 ms) | 6.00 ms (p95 6.91 ms) | 200.81 ms (p95 204.15 ms) |
| F1 | now | 10581 | 23.01 ms (p95 24.13 ms) | 83.10 ms (p95 86.12 ms) | 192.69 ms (p95 213.87 ms) |
| F2 | now | 1935 | 18.44 ms (p95 20.52 ms) | 19.54 ms (p95 20.30 ms) | 79.08 ms (p95 81.14 ms) |
| C1 | now | 19 | 13.74 ms (p95 14.35 ms) | 34.73 ms (p95 35.90 ms) | 406.13 ms (p95 412.53 ms) |
| C2 | now | 1 | 101.30 ms (p95 103.30 ms) | 166.23 ms (p95 171.82 ms) | 551.62 ms (p95 567.89 ms) |

#### lookup

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| point | now | 10 | 58 µs (p95 85 µs) | 51 µs (p95 77 µs) | 108 µs (p95 163 µs) |
| point | asOf | 10 | 64 µs (p95 71 µs) | 49 µs (p95 55 µs) | - |
| point | validAt | 10 | 65 µs (p95 71 µs) | 50 µs (p95 148 µs) | - |
| 2-hop | now | 9 | 51 µs (p95 75 µs) | 47 µs (p95 58 µs) | 100 µs (p95 113 µs) |
| 2-hop | asOf | 9 | 65 µs (p95 69 µs) | 47 µs (p95 50 µs) | - |
| 2-hop | validAt | 9 | 63 µs (p95 69 µs) | 45 µs (p95 49 µs) | - |

#### cypher

| query | variant | rows | sqlite | redb |
|---|---|---|---|---|
| 2-hop | now | 9 | 16.43 ms (p95 17.16 ms) | 47.00 ms (p95 48.02 ms) |
| star | now | 827 | 153.73 ms (p95 155.36 ms) | 133.63 ms (p95 140.04 ms) |
| aggregate | now | 19 | 410.74 ms (p95 412.90 ms) | 365.08 ms (p95 374.77 ms) |

#### paths

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| reachability | now | 1 | 36.00 ms (p95 37.00 ms) | 39.02 ms (p95 39.56 ms) | 106.81 ms (p95 109.22 ms) |
| shortest | now | 1 | 1.62 s (p95 1.64 s) | 4.74 s (p95 4.75 s) | - |
| trail 1..3 | now | 1 | 762.14 ms (p95 768.08 ms) | 2.36 s (p95 2.36 s) | - |

#### concurrency

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| 2-hop | 1 threads | - | 12462 q/s | 15755 q/s | 11544 q/s |
| 2-hop | 4 threads | - | 13842 q/s | 30614 q/s | 33143 q/s |
| 2-hop | 16 threads | - | 15400 q/s | 49154 q/s | 66534 q/s |

#### writes

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| insert | watdiv | - | 148 µs (p95 538 µs) | 147 µs (p95 230 µs) | 5 µs (p95 11 µs) |
| replace (churn) | watdiv | - | 185 µs (p95 292 µs) | 203 µs (p95 302 µs) | 15 µs (p95 19 µs) |
| cascade | watdiv | - | 404 µs (p95 2.02 ms) | 578 µs (p95 973 µs) | - |
| supersede | watdiv | - | 260 µs (p95 906 µs) | 369 µs (p95 548 µs) | - |

#### cyclic

| query | variant | rows | sqlite | sqlite-lftj | redb | oxigraph |
|---|---|---|---|---|---|---|
| triangle | uniform | 1 | 968.56 ms (p95 972.38 ms) | 579.84 ms (p95 617.18 ms) | 905.12 ms (p95 917.18 ms) | 7.74 s (p95 7.80 s) |
| diamond | uniform | 1 | 3.65 s (p95 3.68 s) | 2.35 s (p95 2.38 s) | 2.40 s (p95 2.43 s) | 38.07 s (p95 38.69 s) |
| 4-cycle | uniform | 1 | 3.62 s (p95 3.63 s) | 1.47 s (p95 1.50 s) | 2.32 s (p95 2.35 s) | 40.70 s (p95 40.82 s) |
| tailed triangle | uniform | 1 | 955.76 ms (p95 983.58 ms) | 756.60 ms (p95 786.11 ms) | 1.07 s (p95 1.08 s) | 7.73 s (p95 7.76 s) |
| triangle | skewed | 1 | 5.08 s (p95 5.13 s) | 30.11 ms (p95 31.54 ms) | 44.87 ms (p95 45.76 ms) | 57.08 s (p95 57.16 s) |
| diamond | skewed | - | timeout (> 60 s) | timeout (> 60 s) | timeout (> 60 s) | timeout (> 60 s) |
| 4-cycle | skewed | - | timeout (> 60 s) | timeout (> 60 s) | timeout (> 60 s) | timeout (> 60 s) |
| tailed triangle | skewed | 1 | 5.49 s (p95 5.51 s) | 37.13 ms (p95 575.54 ms) | 54.88 ms (p95 56.17 ms) | 57.58 s (p95 57.87 s) |

### Derived comparisons

Each line compares one measurement and names the factor; differences under 10 % are left out.

**redb faster or smaller than sqlite** (17):

- cyclic / triangle (skewed): 113.3×
- cyclic / tailed triangle (skewed): 100.0×
- concurrency / 2-hop (16 threads): 3.2×
- concurrency / 2-hop (4 threads): 2.2×
- load / bulk load (watdiv): 1.7×
- watdiv / L2 (now): 1.7×
- cyclic / 4-cycle (uniform): 1.6×
- cyclic / diamond (uniform): 1.5×
- lookup / 2-hop (validAt): 1.4×
- lookup / 2-hop (asOf): 1.4×
- watdiv / L3 (now): 1.3×
- lookup / point (validAt): 1.3×
- lookup / point (asOf): 1.3×
- concurrency / 2-hop (1 threads): 1.3×
- cypher / star (now): 1.2×
- lookup / point (now): 1.1×
- cypher / aggregate (now): 1.1×

**redb slower or larger than sqlite** (15):

- size / after load (watdiv): 7.5×
- size / after churn (watdiv): 7.5×
- watdiv / F1 (now): 3.6×
- paths / trail 1..3 (now): 3.1×
- paths / shortest (now): 2.9×
- cypher / 2-hop (now): 2.9×
- watdiv / S3 (now): 2.6×
- watdiv / C1 (now): 2.5×
- watdiv / S1 (now): 1.8×
- watdiv / C2 (now): 1.6×
- watdiv / L1 (now): 1.5×
- writes / cascade (watdiv): 1.4×
- writes / supersede (watdiv): 1.4×
- watdiv / S2 (now): 1.2×
- cyclic / tailed triangle (uniform): 1.1×

**redb faster or smaller than oxigraph** (20):

- cyclic / triangle (skewed): 1272.1×
- cyclic / tailed triangle (skewed): 1049.2×
- watdiv / S3 (now): 33.5×
- cyclic / 4-cycle (uniform): 17.5×
- cyclic / diamond (uniform): 15.9×
- watdiv / C1 (now): 11.7×
- cyclic / triangle (uniform): 8.6×
- cyclic / tailed triangle (uniform): 7.2×
- watdiv / L1 (now): 4.9×
- watdiv / F2 (now): 4.0×
- watdiv / S2 (now): 3.6×
- watdiv / S1 (now): 3.3×
- watdiv / C2 (now): 3.3×
- watdiv / L2 (now): 2.8×
- paths / reachability (now): 2.7×
- watdiv / L3 (now): 2.6×
- watdiv / F1 (now): 2.3×
- lookup / 2-hop (now): 2.1×
- lookup / point (now): 2.1×
- concurrency / 2-hop (1 threads): 1.4×

**redb slower or larger than oxigraph** (7):

- load / bulk load (watdiv): 28.9×
- size / after load (watdiv): 27.6×
- writes / insert (watdiv): 27.1×
- size / after churn (watdiv): 16.5×
- writes / replace (churn) (watdiv): 13.7×
- size / after compaction (watdiv): 8.1×
- concurrency / 2-hop (16 threads): 1.4×

**sqlite faster or smaller than oxigraph** (19):

- watdiv / S3 (now): 87.2×
- watdiv / C1 (now): 29.6×
- cyclic / 4-cycle (uniform): 11.3×
- cyclic / triangle (skewed): 11.2×
- cyclic / tailed triangle (skewed): 10.5×
- cyclic / diamond (uniform): 10.4×
- watdiv / F1 (now): 8.4×
- cyclic / tailed triangle (uniform): 8.1×
- cyclic / triangle (uniform): 8.0×
- watdiv / L1 (now): 7.2×
- watdiv / S1 (now): 6.0×
- watdiv / C2 (now): 5.4×
- watdiv / S2 (now): 4.3×
- watdiv / F2 (now): 4.3×
- paths / reachability (now): 3.0×
- lookup / 2-hop (now): 2.0×
- watdiv / L3 (now): 1.9×
- lookup / point (now): 1.9×
- watdiv / L2 (now): 1.7×

**sqlite slower or larger than oxigraph** (7):

- load / bulk load (watdiv): 49.5×
- writes / insert (watdiv): 27.3×
- writes / replace (churn) (watdiv): 12.5×
- concurrency / 2-hop (16 threads): 4.3×
- size / after load (watdiv): 3.7×
- concurrency / 2-hop (4 threads): 2.4×
- size / after churn (watdiv): 2.2×

## Results at 10⁵ statements

Generated from `results-100000.json`.

### Run

- **Machine:** macos aarch64 · Apple M2 Max · 12 threads
- **Scale:** 100352 statements in the WatDiv-style graph, 5 timed repetitions per query after one checked run
- **Seeds:** skewed 7, watdiv 7
- **Versions:** oxigraph: Oxigraph 0.5 on RocksDB; redb: tiramemsu 0.7.0 on redb; sqlite: tiramemsu 0.7.0 on SQLite 3.53.2; sqlite-lftj: tiramemsu 0.7.0 on SQLite 3.53.2
- **Durability:** tiramemsu: Normal on both backends; Oxigraph: its default (RocksDB WAL)
- **Time cap:** 60 s per query; an engine that passes it is shown as a timeout and left out of that query's comparison
- **Command:** `./target/release/tm-bench-backends run --scale 100000 --reps 5 --out results-100000.json`

#### load

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| bulk load | watdiv | - | 7.30 s | 2.93 s | 0.16 s |

#### size

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| after load | watdiv | - | 384 B | 1343 B | 79 B |
| after churn | watdiv | - | 388 B | 1343 B | 82 B |
| after compaction | watdiv | - | - | 1056 B | 135 B |

#### watdiv

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| L1 | now | 634 | 512 µs (p95 660 µs) | 491 µs (p95 636 µs) | 1.70 ms (p95 2.30 ms) |
| L2 | now | 121 | 142 µs (p95 218 µs) | 167 µs (p95 186 µs) | 478 µs (p95 634 µs) |
| L3 | now | 9 | 71 µs (p95 103 µs) | 61 µs (p95 80 µs) | 152 µs (p95 233 µs) |
| S1 | now | 136 | 578 µs (p95 736 µs) | 1.21 ms (p95 1.70 ms) | 2.27 ms (p95 2.86 ms) |
| S2 | now | 310 | 509 µs (p95 821 µs) | 786 µs (p95 924 µs) | 2.28 ms (p95 2.32 ms) |
| S3 | now | 51 | 315 µs (p95 531 µs) | 690 µs (p95 879 µs) | 11.82 ms (p95 12.90 ms) |
| F1 | now | 3874 | 15.14 ms (p95 15.28 ms) | 13.08 ms (p95 13.10 ms) | 42.91 ms (p95 43.74 ms) |
| F2 | now | 208 | 1.12 ms (p95 1.57 ms) | 1.86 ms (p95 2.08 ms) | 5.62 ms (p95 5.85 ms) |
| C1 | now | 20 | 1.32 ms (p95 1.62 ms) | 5.08 ms (p95 5.32 ms) | 29.30 ms (p95 29.57 ms) |
| C2 | now | 1 | 5.15 ms (p95 6.43 ms) | 12.98 ms (p95 14.46 ms) | 38.71 ms (p95 39.23 ms) |

#### lookup

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| point | now | 10 | 62 µs (p95 206 µs) | 51 µs (p95 107 µs) | 87 µs (p95 141 µs) |
| point | asOf | 10 | 71 µs (p95 136 µs) | 50 µs (p95 58 µs) | - |
| point | validAt | 10 | 65 µs (p95 76 µs) | 48 µs (p95 50 µs) | - |
| 2-hop | now | 8 | 69 µs (p95 153 µs) | 47 µs (p95 87 µs) | 110 µs (p95 134 µs) |
| 2-hop | asOf | 8 | 67 µs (p95 113 µs) | 47 µs (p95 51 µs) | - |
| 2-hop | validAt | 8 | 63 µs (p95 71 µs) | 44 µs (p95 48 µs) | - |

#### cypher

| query | variant | rows | sqlite | redb |
|---|---|---|---|---|
| 2-hop | now | 8 | 1.80 ms (p95 2.17 ms) | 5.33 ms (p95 5.39 ms) |
| star | now | 68 | 14.79 ms (p95 15.27 ms) | 13.32 ms (p95 14.01 ms) |
| aggregate | now | 20 | 27.03 ms (p95 29.26 ms) | 32.47 ms (p95 35.79 ms) |

#### paths

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| reachability | now | 1 | 2.97 ms (p95 3.22 ms) | 3.45 ms (p95 3.90 ms) | 6.94 ms (p95 7.28 ms) |
| shortest | now | 1 | 151.00 ms (p95 152.59 ms) | 476.29 ms (p95 482.04 ms) | - |
| trail 1..3 | now | 1 | 71.48 ms (p95 72.71 ms) | 234.17 ms (p95 235.58 ms) | - |

#### concurrency

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| 2-hop | 1 threads | - | 16789 q/s | 17423 q/s | 18088 q/s |
| 2-hop | 4 threads | - | 17865 q/s | 42123 q/s | 45977 q/s |
| 2-hop | 16 threads | - | 16839 q/s | 55617 q/s | 92949 q/s |

#### writes

| query | variant | rows | sqlite | redb | oxigraph |
|---|---|---|---|---|---|
| insert | watdiv | - | 119 µs (p95 226 µs) | 125 µs (p95 185 µs) | 5 µs (p95 11 µs) |
| replace (churn) | watdiv | - | 138 µs (p95 282 µs) | 178 µs (p95 335 µs) | 14 µs (p95 18 µs) |
| cascade | watdiv | - | 397 µs (p95 1.82 ms) | 447 µs (p95 663 µs) | - |
| supersede | watdiv | - | 276 µs (p95 1.05 ms) | 310 µs (p95 442 µs) | - |

#### cyclic

| query | variant | rows | sqlite | sqlite-lftj | redb | oxigraph |
|---|---|---|---|---|---|---|
| triangle | uniform | 1 | 71.84 ms (p95 71.94 ms) | 47.90 ms (p95 50.14 ms) | 71.17 ms (p95 74.65 ms) | 718.18 ms (p95 730.00 ms) |
| diamond | uniform | 1 | 386.80 ms (p95 407.23 ms) | 196.19 ms (p95 196.43 ms) | 216.90 ms (p95 231.69 ms) | 3.52 s (p95 3.55 s) |
| 4-cycle | uniform | 1 | 369.58 ms (p95 373.32 ms) | 103.43 ms (p95 104.40 ms) | 178.67 ms (p95 185.64 ms) | 3.75 s (p95 3.78 s) |
| tailed triangle | uniform | 1 | 79.00 ms (p95 81.84 ms) | 58.53 ms (p95 59.23 ms) | 87.97 ms (p95 90.82 ms) | 718.26 ms (p95 732.75 ms) |
| triangle | skewed | 1 | 262.40 ms (p95 266.65 ms) | 7.24 ms (p95 8.29 ms) | 9.68 ms (p95 10.11 ms) | 3.44 s (p95 3.45 s) |
| diamond | skewed | 1 | 5.86 s (p95 5.87 s) | 7.11 s (p95 7.52 s) | 5.84 s (p95 6.12 s) | 48.70 s (p95 48.77 s) |
| 4-cycle | skewed | 1 | 4.29 s (p95 4.29 s) | 5.86 s (p95 5.90 s) | 3.91 s (p95 4.31 s) | 50.27 s (p95 50.34 s) |
| tailed triangle | skewed | 1 | 289.77 ms (p95 292.07 ms) | 8.80 ms (p95 8.85 ms) | 12.55 ms (p95 13.27 ms) | 3.44 s (p95 3.46 s) |

### Derived comparisons

Each line compares one measurement and names the factor; differences under 10 % are left out.

**redb faster or smaller than sqlite** (16):

- cyclic / triangle (skewed): 27.1×
- cyclic / tailed triangle (skewed): 23.1×
- concurrency / 2-hop (16 threads): 3.3×
- load / bulk load (watdiv): 2.5×
- concurrency / 2-hop (4 threads): 2.4×
- cyclic / 4-cycle (uniform): 2.1×
- cyclic / diamond (uniform): 1.8×
- lookup / 2-hop (now): 1.5×
- lookup / 2-hop (asOf): 1.4×
- lookup / 2-hop (validAt): 1.4×
- lookup / point (asOf): 1.4×
- lookup / point (validAt): 1.4×
- lookup / point (now): 1.2×
- watdiv / L3 (now): 1.2×
- watdiv / F1 (now): 1.2×
- cypher / star (now): 1.1×

**redb slower or larger than sqlite** (18):

- watdiv / C1 (now): 3.9×
- size / after load (watdiv): 3.5×
- size / after churn (watdiv): 3.5×
- paths / trail 1..3 (now): 3.3×
- paths / shortest (now): 3.2×
- cypher / 2-hop (now): 3.0×
- watdiv / C2 (now): 2.5×
- watdiv / S3 (now): 2.2×
- watdiv / S1 (now): 2.1×
- watdiv / F2 (now): 1.7×
- watdiv / S2 (now): 1.5×
- writes / replace (churn) (watdiv): 1.3×
- cypher / aggregate (now): 1.2×
- watdiv / L2 (now): 1.2×
- paths / reachability (now): 1.2×
- writes / cascade (watdiv): 1.1×
- writes / supersede (watdiv): 1.1×
- cyclic / tailed triangle (uniform): 1.1×

**redb faster or smaller than oxigraph** (21):

- cyclic / triangle (skewed): 355.0×
- cyclic / tailed triangle (skewed): 273.9×
- cyclic / 4-cycle (uniform): 21.0×
- watdiv / S3 (now): 17.1×
- cyclic / diamond (uniform): 16.3×
- cyclic / 4-cycle (skewed): 12.9×
- cyclic / triangle (uniform): 10.1×
- cyclic / diamond (skewed): 8.3×
- cyclic / tailed triangle (uniform): 8.2×
- watdiv / C1 (now): 5.8×
- watdiv / L1 (now): 3.5×
- watdiv / F1 (now): 3.3×
- watdiv / F2 (now): 3.0×
- watdiv / C2 (now): 3.0×
- watdiv / S2 (now): 2.9×
- watdiv / L2 (now): 2.9×
- watdiv / L3 (now): 2.5×
- lookup / 2-hop (now): 2.4×
- paths / reachability (now): 2.0×
- watdiv / S1 (now): 1.9×
- lookup / point (now): 1.7×

**redb slower or larger than oxigraph** (7):

- writes / insert (watdiv): 23.7×
- load / bulk load (watdiv): 17.9×
- size / after load (watdiv): 16.9×
- size / after churn (watdiv): 16.4×
- writes / replace (churn) (watdiv): 12.7×
- size / after compaction (watdiv): 7.8×
- concurrency / 2-hop (16 threads): 1.7×

**sqlite faster or smaller than oxigraph** (21):

- watdiv / S3 (now): 37.5×
- watdiv / C1 (now): 22.2×
- cyclic / triangle (skewed): 13.1×
- cyclic / tailed triangle (skewed): 11.9×
- cyclic / 4-cycle (skewed): 11.7×
- cyclic / 4-cycle (uniform): 10.1×
- cyclic / triangle (uniform): 10.0×
- cyclic / diamond (uniform): 9.1×
- cyclic / tailed triangle (uniform): 9.1×
- cyclic / diamond (skewed): 8.3×
- watdiv / C2 (now): 7.5×
- watdiv / F2 (now): 5.0×
- watdiv / S2 (now): 4.5×
- watdiv / S1 (now): 3.9×
- watdiv / L2 (now): 3.4×
- watdiv / L1 (now): 3.3×
- watdiv / F1 (now): 2.8×
- paths / reachability (now): 2.3×
- watdiv / L3 (now): 2.1×
- lookup / 2-hop (now): 1.6×
- lookup / point (now): 1.4×

**sqlite slower or larger than oxigraph** (7):

- load / bulk load (watdiv): 44.7×
- writes / insert (watdiv): 22.5×
- writes / replace (churn) (watdiv): 9.8×
- concurrency / 2-hop (16 threads): 5.5×
- size / after load (watdiv): 4.8×
- size / after churn (watdiv): 4.7×
- concurrency / 2-hop (4 threads): 2.6×
