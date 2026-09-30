# Benchmarks

`cargo bench -p tiramemsu --bench <core|sparql|path>`. No targets are asserted; the numbers below
are recorded to steer tuning (`lat.md/roadmap#Benchmarks`).

## path (add-path-engine, M3)

`PATH_BENCH_STATEMENTS` sets the power-law graph (default 100 000 statements, 5 statements per
node, hub-skewed targets; `1000000` is the roadmap size and `10000000` the large one). Run on an
Apple laptop, bundled SQLite, `--warm-up-time 1 --measurement-time 2`, 100 000 statements.

| Group | Shape | Now | AsOf | ValidAt |
|---|---|---|---|---|
| `shortest` | `link+` ANY_SHORTEST from a random start, at most 6 hops | 5.9 ms | 6.5 ms | not run |
| `trail3` | `link{1,3}` TRAIL from a random start | 118 µs | 145 µs | 144 µs |
| `trail3` | `sys:anyRelationship{1,3}` TRAIL | 95 µs | 134 µs | 95 µs |
| `reach` | `next+` REACH over a 100 000-hop chain | 1.38 s | | |
| `reach` | `near+` REACH over a 50 000-node small world | 38 ms | | |
| `reach` | `step+` ALL_SHORTEST over a 30 x 30 grid (12 hops) | 10 ms | | |

A long chain is the worst case: every hop is one layer and so one statement (about 14 µs per
layer). Wide frontiers amortise the statement cost over up to a whole chunk.

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
