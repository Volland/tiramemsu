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
