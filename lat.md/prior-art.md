# Prior Art

Systems studied for the design, with what was taken from each and what was avoided. The full comparison with sources is in `docs/design-options.md`.

## MillenniumDB

A research graph database (GPL-2.0) whose domain graph gives every edge an id, with tagged 64-bit ObjectIds, B+tree permutations, leapfrog triejoin and automaton-based path search.

- **Taken:** edge ids as first-class objects ([[data-model#Statements]]); inline small values in a tagged id ([[data-model#ObjectId]]); permutation indexes ([[storage#Schema]]); product-automaton paths ([[query#Physical Planning#Path Engine]]); LFTJ as a later option ([[query#Physical Planning#LFTJ]]).
- **Changed:** the tag moves to the low bits for SQLite's signed varints; the separate Properties table is dropped, so properties have ids too; one store serves SPARQL and Cypher, where MillenniumDB keeps separate physical models.

## Datomic

Immutable datoms `[e a v tx added?]`, transactions as entities, and `as-of`, `since`, `history` and `with` database values. Transaction time only.

- **Taken:** reified transactions; the four views; `with` as speculation ([[time-model#Speculative Transactions]]); cardinality one ([[time-model#Operations#Cardinality One]]); the log ([[time-model#Event Log]]).
- **Changed:** statement-level identity (datoms have none); valid time added ([[time-model#Valid Time]]); no excision ([[time-model#Never Forget]]).

## CozoDB

A Datalog database on SQLite and RocksDB. Its time travel orders versions newest-first per key and seeks straight to the version for a time, which keeps most throughput under heavy history.

**Taken:** `t_add DESC` in the history indexes, so as-of seeks land on the right version directly ([[storage#Schema]]).

## Fluree

An RDF database (BUSL-1.1) with SPARQL, openCypher, RDF 1.2 edge annotations and time travel. It keeps history in a sidecar, so current-state scans skip old versions.

**Taken:** keeping history out of the hot path, done here with partial indexes ([[storage#Triple Table]]); annotations as durable facts on edges.

## Graphiti

An agent-memory temporal knowledge graph (Apache-2.0) whose edges carry both world validity and system time. A contradiction closes the old edge's validity window.

**Taken:** valid time as a first-class concern for agent memory ([[time-model#Valid Time]]).

## Neptune OneGraph

Amazon's 1G model makes every statement first-class with an id, so RDF triples, LPG edges and LPG properties are all statements that can carry further statements.

**Taken:** the uniform statement model and the RDF/LPG mapping ([[data-model#Vocabulary Mapping]]). Its hard problem, global IRIs versus local LPG ids, is solved here by skolem IRIs ([[data-model#Nodes and Identity]]).

## oxilite

An Oxigraph-compatible RDF database by the same author (MIT/Apache-2.0) that runs SPARQL, Cypher and Datalog on SQLite. It is a shipped system that has already solved much of what tiramemsu needs.

It has a core with no I/O of its own over rusqlite, dlopen, Turso, Cloudflare D1 and wasm. Other features: hashed tagged term ids, `quads` with three covering permutations, an own statistics-driven join order, Cypher lowered to SPARQL algebra, and optional versioning by an append-only change log beside the present table. It passes the W3C suites and 96 % of the openCypher TCK.

- **Taken:** statistics as a precondition for SQLite join ordering ([[query#Physical Planning#Join Ordering]]); a declared-capability executor boundary ([[architecture#Executor]]); its hand-written Cypher parser as the fallback ([[query#Front Ends#Cypher]]); allow-list test harnesses in which every expected deviation is listed with a reason and any unexpected pass fails; the `write-cost` and `as-of-latency` benchmarks ([[roadmap#Benchmarks]]); `SERVICE` rather than `GRAPH` for version scoping ([[query#Temporal Syntax]]); the retrieval design ([[roadmap#Milestones]]).
- **Changed:** statement identity instead of quads with optional reifiers ([[data-model#Statements]]); lifetime columns and covering history indexes instead of a change log, so the past is one index range and not a log probe ([[storage#Triple Table]]); valid time, which oxilite lacks ([[time-model#Valid Time]]); dictionary counters instead of hashes, because the single embedded writer can afford a lookup and small ids keep varints short; one IR with semantic flags instead of lowering Cypher to SPARQL.
- **Not taken:** D1 as a target (no interactive transactions), and oxilite's purge, which rewrites history. Tiramemsu erases by crypto-shredding ([[time-model#Erasure]]).

## Others

Other systems were studied and informed smaller choices or gave warnings about what to avoid.

- **Oxigraph:** canonical term encoding, the `spargebra` parser ([[query#Front Ends#SPARQL]]).
- **Apache AGE:** Cypher → SQL translation plus a native variable-length operator ([[query#Physical Planning]]).
- **XTDB v2:** bitemporal columns, and keeping current state separate from history.
- **TerminusDB, Dolt:** git-style branching, deliberately not adopted ([[time-model#Speculative Transactions]]).
- **Kùzu (archived 2025), CozoDB, cr-sqlite:** single-sponsor embedded databases stall, so the core is kept small and standard-tracking.
- **simple-graph:** edges without ids and recursive-CTE traversal, both avoided.
