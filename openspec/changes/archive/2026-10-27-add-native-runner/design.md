## Context

`tm-exec` lowers the IR to SQL, with three native islands (paths, LFTJ, recall) exposed to SQLite as table functions. On redb nothing can host SQL, so the whole IR runs natively. The join literature the design interview reviewed (2026-10-27):

- **MillenniumDB:** LFTJ over B+tree permutations for every basic graph pattern.
- **Kùzu:** binary hash joins for acyclic parts, multiway intersection for cycles, chosen by a cost-based optimizer.
- **Umbra** (Freitag et al., VLDB 2020): hash tries built on the fly where a worst-case-optimal join pays.
- **Free Join** (Wang et al., SIGMOD 2023): one plan form unifying both, with lazily built column-oriented tries.
- **RDF-3X:** dynamic-programming join order over six permutations, merge joins and sideways information passing.
- **Oxigraph:** volcano iterators with nested-loop and hash joins.

## Decisions

### D-1. Hybrid joins chosen per step

The planner first splits a basic graph pattern (with its filters pushed down) into its acyclic skeleton and its cyclic cores (the 2-connected components of the query graph with more than two patterns). The join kind is chosen per step:

- **Index nested loop:** when the next pattern's bound positions (from the left side) form a permutation prefix and the estimated left size is small (≤ 4 096 rows), probe the store per left row, sorted by the probe key, so cursor seeks move forward.
- **Hash join:** otherwise. Build on the smaller estimated side; the build side's key set becomes a Bloom filter pushed into the probe side's scan (sideways information passing).
- **Generic join:** for each cyclic core, a multiway intersection over variables in an order chosen by estimated selectivity. Each pattern of the core is an access path on the permutation whose prefix follows the variable order (six live permutations make every order available). Intersection uses galloping seeks on cursors; a range that will be intersected more than once is materialised into a sorted `Vec<u64>` trie level.

- motivated_by:: Kùzu and Umbra precedent; the worst-case-optimal requirement of the interview; [[query#Physical Planning#LFTJ]] results (12× on skewed triangles).

### D-2. Cardinality estimation from exact statistics

The single-pattern cardinality is exact from `pred_stats` when one position is bound and the predicate is, or from a bounded probe (≤ 4 096 keys) when two are. Join size uses the distinct counts (`|R|·|S| / max(d_R, d_S)`). Dynamic programming over connected subsets runs up to 12 patterns; above that, a greedy order from the most selective pattern.

### D-3. Batches of ids, decode late

Operators exchange batches of up to 1 024 rows of `u64` columns. Filters that compare ids (equality, `sameTerm`, graph membership, time scope) run on ids; filters and expressions that need values decode through a per-query cache over the term dictionary. Projection decodes once at the end. Inline values (numbers, dates, short strings) decode without a dictionary read.

### D-4. One function library

The Rust bodies of the SQL functions registered by `tm-exec` (`udf_fn.rs`: comparison, arithmetic, string, date, hashing, casting, language and datatype functions, with SPARQL error semantics) move to a module that does not depend on SQLite, and both runners call them. Same functions, same semantics.

### D-5. Paths and recall reuse their engines

The path automaton and search in `tm-exec/src/path/` take a `NeighbourFetch` trait (today implemented with SQL in `fetch.rs`); `tm-native` implements it with permutation scans. Text, vector and hybrid recall call the `tm-store` recall functions.

### D-6. Morsel parallelism

A scan estimated above 50 000 rows is split into key-range morsels (by sampling the first key byte range) executed on a rayon pool; hash builds use partitioned parallel inserts. The threshold is `OpenOptions::parallel_threshold`; 0 disables parallelism. Result order is restored where the IR requires it.

### D-7. Semantics are differential, not reinvented

Every operator's semantics is checked against `tm-exec` on SQLite with the same data: the dialect corpus, the W3C SPARQL suite subset that `tm-exec` passes, the Cypher TCK subset, the recipes, the path and recall suites, and a random-query property test.

## Constraints

- **Result equality:** the native runner must produce what `tm-exec` produces, including bag multiplicities, `ORDER BY` stability rules and error kinds. Constrains [[openspec:native-query-execution#Same answers as the SQL runner]] {strength: must}.
- **Budgets:** `QueryBudget` semantics from [[query#Query Budgets]] apply unchanged.

## Risks / Trade-offs

- **Size:** an executor for the whole IR is the largest piece of the project. The differential suites are the guardrail.
- **Estimation errors** on correlated predicates: hash joins with Bloom filters bound the damage of a bad order better than nested loops; explain shows the estimates.
- **Thread pool in a library:** the pool is created lazily and sized by `available_parallelism`; embedders can turn it off.

## Resolution

Implemented 2026-10-27.

- **Planner reuse.** `tm-native` plans with `tm-exec`'s `Planner` through `PlanSource` and decodes with its `Decoder` through `TermLoad`; the evaluator mirrors `Gen`'s SQL semantics (domains, three-valued logic, the UDF bodies reached through `tm-exec`'s registries rather than moved, D-4).
- **Join order (D-2):** greedy from exact predicate statistics and per-position distinct counts. Dynamic programming was not needed on the corpus and is deferred.
- **Joins (D-1):** index nested loop or hash join per step from the estimate; no Bloom filters yet. Cyclic cores use a generic join over hash tries built once per pattern (Umbra style), not galloping cursor seeks; explain reports the keys it visited, and the skewed-triangle test bounds them by the AGM bound.
- **Rows (D-3):** steps exchange materialised rows of ids, not fixed 1 024-row batches; decoding is late as designed.
- **Filter placement** (not in the design): a filter over a join is split into conjuncts, the patterns into connected components, and each conjunct is applied on the first relation that fixes every variable it reads, then before paths. Without it a disconnected pattern with a filter built the whole cross product first.
- **Parallelism (D-6):** join steps (index-nested-loop probes and hash probes) above `parallel_threshold` (default 4 096 rows) run as 1 024-row morsels on `std::thread::scope`, not rayon (no new dependency). Scans and hash builds stay sequential. The threshold lives on `QueryEngine::with_parallel_threshold` and `OpenOptions::parallel_threshold`.
- **No `OpenOptions::runner`:** the backend decides the runner (SQLite → SQL, redb → native), which is the user-visible choice `tm create --backend` offers.
- **Explain** runs the query and lists the steps it took; `ReadStore::access_path` names the permutation (`pos live scan`).
- **Budgets:** every join loop polls per emitted row; morsel workers inherit the interrupt; the facade's query test hook runs on the native route too.
- **Evidence.** `tests/native_runner.rs`: the dialect corpus and 32 extra SPARQL queries over every fixture agree on SQLite and redb; 48 random histories × 4 random queries agree (now and history views). The facade suite with `TIRAMEMSU_DEFAULT_BACKEND=redb` passes 519 tests; the 44 that fail are SQL-specific by construction (they call `read_sql`, open the file with rusqlite, rely on the SQLite reader pool or the SQL LFTJ route) or use saved answers, whose tables are still SQL-only on redb (`move-sqlite-onto-store`).
- **Build:** redb and `tm-redb` compile at `opt-level = 3` in dev builds; unoptimised, redb loads ten times slower and dominated the test time.
