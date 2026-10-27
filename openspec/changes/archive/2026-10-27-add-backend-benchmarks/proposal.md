# Backend benchmarks: redb, SQLite and Oxigraph

Target release: **0.8 — Two engines**. Fifth of six changes. Depends on `add-native-runner` and `add-tm-cli`.

## Why

The redb backend exists to be faster on reads, especially on joins, and to scale with concurrent readers. That claim needs evidence that is fair, reproducible and honest about where redb loses: writes, file size and point reads are expected weak spots (`docs/storage-engine-research.md`). Oxigraph is the reference Rust RDF store, so it is the external baseline for the workloads it supports.

## What Changes

- **A benchmark crate `bench/backends`** (outside the default workspace build, because Oxigraph pulls in RocksDB) with a driver that loads the same N-Triples into tiramemsu-on-SQLite, tiramemsu-on-redb and Oxigraph (RocksDB store), checks equal result counts for every query, then times it.
- **Workloads:**
  - WatDiv-style templates (linear, star, snowflake, complex) on a generated dataset at 10⁵, 10⁶ and 10⁷ triples;
  - LSQB-style cyclic subgraph queries (triangle, 4-clique, diamond, cycle with a tail) on uniform and skewed graphs, SQLite with LFTJ off and on;
  - point and 2-hop lookups in the now, `asOf` and `validAt` views (Oxigraph: now only);
  - concurrent mixed reads at 1, 4 and 16 threads;
  - writes: bulk load, single-statement transactions, churn, cascade and supersede;
  - Cypher on both tiramemsu backends;
  - paths (reachability, shortest path, trail);
  - file size in bytes per statement after load and after churn, and after compaction.
- **Report:** `bench/backends/README.md` with the machine, versions, durability settings, medians and p95, and a section of strong and weak sides per backend. The report states the project's position: tiramemsu prefers transactional guarantees (one ACID transaction per write, never-forget history, crash consistency) and security (no unsafe code, no network surface, typed errors) over raw speed, and the numbers are read in that light.
- **`tm bench`** runs a reduced suite on the user's machine.
- **Articles** in `article/redb-backend/`: "Two engines, one memory" (architecture) and "Worst-case-optimal joins on a key-value store" (joins and benchmark results), plus a site page.

## Capabilities

### New Capabilities

- `backend-benchmarks`: the harness, its fairness rules and the report.

### Modified Capabilities

None.

## Impact

- **New:** `bench/backends/` (crate, generators, query sets, results), `article/redb-backend/`, `site/articles/` page.
- **CLI:** `tm bench` subcommand.
- **lat.md:** `roadmap.md#Benchmarks` (backend comparison), `prior-art.md` (Oxigraph section, Kùzu, Free Join, Umbra), `tests.md`.
- **Requirements added:** [[openspec:backend-benchmarks#Equal results before timing]], [[openspec:backend-benchmarks#Workload coverage]], [[openspec:backend-benchmarks#Honest report]], [[openspec:backend-benchmarks#Reproducible runs]].

## Non-goals

- No benchmark gate in CI in this change (the scale-benchmark change owns gates).
- No tuning of Oxigraph beyond its documented bulk loader and defaults.
