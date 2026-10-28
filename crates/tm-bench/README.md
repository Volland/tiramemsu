# tm-bench

The benchmark harness that `tm bench` and `bench/backends` use to compare [tiramemsu](https://github.com/Volland/tiramemsu) on SQLite against tiramemsu on redb, and against Oxigraph.

- **Data:** seeded generators. One is a WatDiv-style e-commerce graph (users, products, retailers and reviews, with skewed follows, likes and review targets). The others are the triangle benchmark's uniform and hub-and-spoke graphs.
- **Workloads:**
  - bulk load and file size (after load, after churn, after compaction);
  - WatDiv-style linear, star, snowflake and complex queries;
  - LSQB-style cyclic counts (triangle, diamond, 4-cycle, tailed triangle);
  - point and 2-hop lookups in the now, as-of and valid-at views;
  - concurrent readers at 1, 4 and 16 threads;
  - writes: insert, replace, cascade and supersede;
  - Cypher;
  - paths.
- **N3 reasoning** (feature `n3`, module `reason`): `View::reason` time against store size (10 000, and 100 000 with `--full`) and distinct validity bounds (1 and 16, and 256 with `--full`): a pruned two-hop join swept pointwise, the same rule atemporal, and an unpruned load of the whole store.
- **Fairness:** every query runs once on every engine and the results are compared before anything is timed: counts always, and sorted rows for results of up to 1 000 rows. A mismatch stops the run and names the query and the engines.
- **Results:** median and p95 per query, values with units, and the seeds, scale, versions, machine and command line, as JSON. `report::markdown` turns a results file into tables and derived strengths and weaknesses. Differences under 10 % are left out.

```sh
tm bench --scale 100000 --reps 5 --out results.json                         # SQLite and redb
cargo run --release -p tm-bench --features n3 --example reason [-- --full]  # N3 reasoning cost
cd bench/backends && cargo run --release -- run --scale 100000 --reps 5      # plus Oxigraph
```

Licensed under MIT or Apache-2.0, at your option.
