## Context

Three engines with different feature sets: Oxigraph has no transaction or valid time, no statement ids and no Cypher. Workloads are therefore split by what each engine can answer, and each comparison says which engines took part.

## Decisions

### D-1. Generated data, published generators

WatDiv's original generator is a C++ tool; the harness reimplements its schema shape (users, products, reviews, retailers, with skewed degree distributions) as a seeded Rust generator, and the query templates follow WatDiv's L, S, F and C families. LSQB-style cyclic graphs come from the triangle benchmark generators (uniform, hub-and-spoke, layered). Seeds are recorded in the results.

### D-2. Fairness rules

- **Inputs:** the same N-Triples input for every engine.
- **Loading:** each engine uses its bulk path (tiramemsu bulk import; Oxigraph `bulk_loader`).
- **Correctness:** the result count, and for small results the sorted solutions, must be equal across engines before a query is timed; a mismatch fails the run.
- **Caches:** warm runs (one discarded warm-up, then N timed) and a cold run after reopening.
- **Durability:** tiramemsu at `Normal` on both backends; Oxigraph at its default (RocksDB WAL), stated in the report.
- **Statistics:** median and p95 of per-query latency; throughput for concurrency; bytes per statement.

### D-3. Honest report

The report lists every workload, including those redb loses, and ends with a strengths-and-weaknesses table per engine. Headline wording is only allowed for a claim that holds on most read workloads at 10⁶ statements and above.

## Risks / Trade-offs

- RocksDB build time and size: the crate is excluded from the default workspace build and from CI tests.
- Machine variance: the report records the machine and runs each configuration at least five times.

## Resolution

Implemented 2026-10-27.

- **Two crates.** The generators, workloads, checks, results and report are the workspace crate `tm-bench`, so `tm bench` runs the SQLite-and-redb part without RocksDB. `bench/backends` is its own workspace and adds the Oxigraph engine. Oxigraph needs its `rdf-12` feature, because tiramemsu turns on RDF 1.2 in `oxrdf` and `spargebra`. `tm-sparql` gained explicit `Unsupported` fallbacks for the spargebra syntax Oxigraph turns on (`LATERAL`, `ADJUST`).
- **Time cap (not in the design).** The first 10⁵ run stalled for over ten minutes on Oxigraph's skewed diamonds. Each query now has a 60 s cap: tiramemsu stops it with a query budget and Oxigraph with its `CancellationToken`. An engine that passes the cap is recorded as a timeout, left out of that query's comparison, and counted at the cap in the derived comparisons (shown as a lower bound).
- **The benchmark found four native-runner bugs**, fixed before the final runs (`lat.md/query.md#Native Runner`):
  - a path endpoint treated as a path output (a shortest path that timed out);
  - a 100 000-key cardinality probe (a 32× slower as-of lookup);
  - Cypher relationship patterns not joined as patterns (8×);
  - filters applied only after the whole join (6×).
- **Fairness rules followed:** same triples, bulk paths, checked results, one checked run and five timed runs, medians and p95, durability stated. The cold run after reopening (D-2) was not measured.
- **Scale.** 10⁵ and 10⁶ were run; 10⁷ was not, because the 10⁶ run took 39 minutes, much of it on cyclic queries that pass the cap, and 10⁷ would multiply that. The report records the machine (Apple M2 Max), versions, seeds and command.
- **Data note.** The WatDiv L2 constant selects no retailer at 10⁶ (0 rows on every engine); it is kept so that the two scales run the same queries.
