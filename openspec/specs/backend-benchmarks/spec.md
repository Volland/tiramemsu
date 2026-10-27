# backend-benchmarks Specification

## Purpose
The backend benchmark compares tiramemsu on SQLite and on redb with Oxigraph on generated data, checks equal results before timing anything, covers reads, joins, paths, writes, concurrency and size, and reports strengths and weaknesses with the inputs needed to reproduce a run.

## Requirements

### Requirement: Equal results before timing
The benchmark driver SHALL check, for every query and every engine taking part, that the result count (and for results of at most 1 000 rows, the sorted solutions) equals the other engines' before timing it, and SHALL fail the run on a mismatch.

#### Scenario: Mismatch fails the run
- **WHEN** one engine returns a different count for a query
- **THEN** the driver stops with an error naming the query and the engines, and records no timing for it

### Requirement: Workload coverage
The harness SHALL measure WatDiv-style linear, star, snowflake and complex queries, LSQB-style cyclic queries on uniform and skewed graphs, point and 2-hop lookups in every view, concurrent reads at 1, 4 and 16 threads, bulk and transactional writes, churn, cascade and supersede, Cypher, paths and file size, on the engines that support each workload.

#### Scenario: Small-scale smoke run
- **WHEN** the harness runs at 10⁴ triples with one repetition
- **THEN** it produces a results file with an entry for every workload and engine pair that the workload supports

### Requirement: Honest report
The report SHALL show every measured workload with median and p95, name the machine, versions and durability settings, include the workloads where redb is slower, and summarise the strengths and weaknesses of each engine, stating the project's preference for transactional guarantees and security over raw speed.

#### Scenario: Report lists losses
- **WHEN** redb is slower than SQLite on a workload
- **THEN** that workload appears in the report's weaknesses for redb

### Requirement: Reproducible runs
Every results file SHALL record the generator seeds, scale, engine versions, machine description and command line, so that the same command reproduces the run.

#### Scenario: Results carry their inputs
- **WHEN** a results file is written
- **THEN** it contains the seeds, scale, versions, machine and command
