# Triangles: SQLite nested loops versus an intersection join

`bench.py` counts triangles `a→b→c→a` over tiramemsu's `triple` schema (three live covering indexes, `ANALYZE`d) and compares SQLite's nested-loop plan with a worst-case-optimal style join written in Python: bind `x`, then `y` in `out(x)`, then intersect `out(y)` with `in(x)`. It is the evidence behind the "Triangles" benchmark in `lat.md/roadmap.md`. It needs only the standard library.

```sh
python3 bench.py
```

Results on an Apple M2 Max (SQLite 3.45.3 through Python, 2026-09-30):

| Graph | Edges | Triangles | SQLite | Intersection join |
|---|---|---|---|---|
| uniform, out-degree 5 | 500 000 | 126 | 564 ms | 565 ms |
| 300 hubs × 1 500 spokes, both ways, plus noise | 1.1 M | 193 423 | 79.6 s | 1.9 s |
| 3 layers of 150, 3 closing edges each | 45 450 | 202 500 | 490 ms | 14 ms |
| 3 layers of 300 | 180 900 | 810 000 | 3.8 s | 55 ms |
| 3 layers of 600 | 721 800 | 3.24 M | 30.3 s | 292 ms |

The intersection join is plain Python, so the gap is an underestimate of what native code would show. It is one machine and hand-made graphs, not a real data set.

## Through tiramemsu: SQL route versus the LFTJ operator

`crates/tiramemsu/examples/triangles.rs` runs the same kinds of graphs through tiramemsu itself (OpenSpec change `add-lftj-operator`): it loads one database file, opens it twice (default options, and `planner.lftj` enabled with `min_rows_estimate: 0`), checks that the native handle really routes to `tm_lftj`, and **counts triangles on both routes first, stopping with an error if the counts differ**. Only then does it time a `COUNT(*)` over `?a v:k ?b . ?b v:k ?c . ?c v:k ?a` on each route (best of 3). `tests/lftj_triangles.rs` runs the same harness on tiny graphs in the test suite.

```sh
cargo run --release -p tiramemsu --example triangles            # small (seconds)
cargo run --release -p tiramemsu --example triangles -- full    # the bench.py sizes
```

Results at the small scale on an Apple M2 Max (release build, bundled SQLite, 2026-10-03):

| Graph | Edges | Triangles | SQL | LFTJ | SQL / LFTJ |
|---|---|---|---|---|---|
| uniform, out-degree 5 | 25 000 | 126 | 30.2 ms | 23.4 ms | 1.3× |
| 30 hubs × 150 spokes, both ways, plus noise | 13 960 | 620 | 143.3 ms | 11.8 ms | 12.1× |
| 3 layers of 40, 3 closing edges each | 3 317 | 14 040 | 12.5 ms | 5.7 ms | 2.2× |
| 3 layers of 80, 3 closing edges each | 13 034 | 56 160 | 88.2 ms | 23.3 ms | 3.8× |

Counts were identical on both routes for every graph. The pattern matches each directed triangle once per rotation, so these counts are three times the distinct triangles (`bench.py` counts the same way). The `full` scale was not run for this change: the operator materialises its output rows before SQLite counts them, so the layered graphs of 300 and 600 (0.8 M and 3.2 M output rows) hold hundreds of megabytes during the count, and those numbers are still to be measured. The LFTJ times include each pattern's ordered scan of `triple`; the default `min_rows_estimate` of 100 000 keeps small regions like these on SQL unless lowered.
