# SQLite vs DuckDB on the tiramemsu workload

`bench.py` runs the same data and queries on SQLite and DuckDB. It is the evidence behind decision D26 in `lat.md/overview.md` (see `lat.md/prior-art.md#DuckDB`).

The data has `N` nodes, which gives about `11 × N` statements. Each node has one `rdf:type` (90 % of nodes in one class), a name with four retracted older names, and five `knows` edges. One node in 2 000 has a rare `ceo` edge.

```sh
python3 -m venv venv && ./venv/bin/pip install duckdb
./venv/bin/python bench.py 100000     # 1.1 M statements
./venv/bin/python bench.py 1000000    # 11 M statements
```

The two result files, `results-100000.json` (1.1 M statements) and `results-1000000.json` (11 M), are from an Apple M2 Max, with DuckDB 1.5.6 and SQLite 3.45.3 (Python's bundled build), on 2026-09-29.

- SQLite uses the schema from `lat.md/storage.md#Schema`: nine indexes, WAL mode, and `ANALYZE`.
- DuckDB has no partial or composite-lookup indexes, so it gets single-column ART indexes on `s` and `o`, one on `(p, o)`, and a primary key on `eid`.
- Both engines run through their Python bindings. The bindings add a fixed cost to every DuckDB call: `SELECT 1` takes 66 µs, against 0.9 µs for SQLite.
