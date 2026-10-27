# Worst-case-optimal joins on a key-value store

*How tiramemsu's native runner joins SPARQL and Cypher patterns over redb: what we borrowed from RDF-3X, Kùzu and Umbra, why cyclic patterns need a different join, and the four performance bugs the benchmark found in our own runner.*

---

A graph query is a join. `?a :knows ?b . ?b :knows ?c` is the `knows` table joined with itself on `?b`. On SQLite, tiramemsu compiles that join to SQL, and SQLite's planner chooses nested loops over our indexes. On redb there is no planner: a key-value store gives you ordered keys and range scans, nothing more. So the redb engine needed a runner of its own, and building one meant choosing how to join.

## How other engines join

We read how the engines we respect do it before writing any code.

**RDF-3X** keeps all six orders of a triple (`spo`, `sop`, `pso`, `pos`, `osp`, `ops`), so any bound prefix is one range scan. It orders joins by dynamic programming over exact statistics. **Jena TDB2** and **Oxigraph** keep three to nine permutations in B-trees or RocksDB and run index nested loops in a heuristic order. **MillenniumDB** uses leapfrog triejoin, a worst-case-optimal join, for every basic graph pattern. **Kùzu** runs hash joins for acyclic parts and switches to a worst-case-optimal multiway join where the query has cycles. **Umbra** (Freitag et al., VLDB 2020) adds hash tries inside an otherwise binary plan, used only where an intermediate result would grow. **Free Join** (Wang et al., SIGMOD 2023) shows that binary and generic joins are two points of one plan space.

The point of the worst-case-optimal family is cycles. Take the triangle `?a :k ?b . ?b :k ?c . ?c :k ?a` on a graph with a few hubs, each linked to thousands of spokes. Any binary plan first joins two of the three edges, and through a hub that intermediate result is hubs × spokes², large even when the answer is tiny. A generic join instead binds one variable at a time and intersects the candidate sets of every pattern that mentions it, so its work is bounded by the largest possible output (the AGM bound), not by an intermediate.

Most agent-memory queries are not cycles, though. They are stars ("everything about this person") and chains ("who works where, located where"). On those a generic join gains nothing and pays for building its tries. So we took the Kùzu and Umbra design: binary joins by default, a generic join for cyclic cores.

## The runner

`tm-native` evaluates the same logical plan our SQL generator compiles, so SPARQL, Cypher, the planner, the path engine and result decoding are shared, and only execution differs:

- **Storage:** six live permutations and three history permutations in redb, with exact per-predicate statistics (live statements, distinct subjects, distinct objects) maintained in the same write transaction.
- **Order:** greedy, from those statistics, preferring patterns connected to variables already bound.
- **Acyclic steps:** an index nested loop (one prefix scan per input row on the permutation that fits) or a hash join (one scan, hashed on the shared variables), whichever reads less by the estimate.
- **Cyclic cores:** a generic join over hash tries built once per pattern. Explain reports the keys it visited, and a test checks that count against the AGM bound of a hub-and-spoke fixture.
- **Late decoding:** joins, id equality, graph membership and time filters work on 64-bit ids. Terms are decoded only for the columns a query returns. A `COUNT(*)` over a two-hop join reads no dictionary entry at all, and a test checks that.
- **Parallel morsels:** a join step with more than 4,096 input rows is cut into morsels of 1,024 rows, run on scoped threads and concatenated in order, so results are identical either way.
- **Budgets:** every join loop polls the deadline and cancellation token per emitted row, so a 100 ms timeout stops even a cross product promptly.

## What the benchmark taught us

We compared the runner with SQLite on the same data, and both with Oxigraph. The first full run at 10⁵ statements was humbling. All three engines agreed on every answer, but four redb timings were wrong by an order of magnitude, and each one traced back to a bug in our own runner.

**A shortest path that never finished.** `MATCH p = shortestPath((a {name: 'User 1'})-[:follows*]->(b {name: 'User 2'}))` took 145 ms on SQLite and passed our 60-second cap on redb. Both endpoints were bound by node patterns, but our filter placement treated the path's end variable as an output of the path. So the `{name: 'User 2'}` test ran after the search, and the runner searched from User 1 to every one of 3,740 nodes before keeping one. The fix: a variable is a path's output only when nothing else binds it. That took it to 0.45 s.

**A time-travel lookup 32× slower than now.** A two-hop lookup as of an earlier transaction took 1.94 ms against 45 µs in the current view, with a perfect four-row plan. A profiler showed 97 % of the time in cardinality estimation: in a historical view the live statistics did not apply, so the estimator counted the matching history rows, up to 100,000 of them per pattern per query. The design said probes stop at 4,096 keys. We also now use the exact live statistics as the estimate in every view, since history is never smaller than the live set. 45 µs.

**Cypher patterns that hid from the join.** The Cypher compiler wraps each relationship pattern in a filter and a projection, and the runner treated those wrappers as opaque inputs, scanning each predicate in full. A two-hop Cypher query took 17 ms against 2.3 ms on SQLite. Now a pattern under filters and a projection that keeps all its variables joins as a pattern, its filters become join conditions, and a small positive `EXISTS` (Cypher's `{name: 'User 7'}` property map) seeds the join with its few values. 2.8 ms.

**Filters applied too late.** WatDiv's F2 joins six patterns and filters on two numbers. SQLite tests each `WHERE` condition in the first loop that binds its columns; our runner tested both after the last join step, so thousands of rows flowed through four joins only to be thrown away. Now a condition is checked inside the join, right after the step that binds its last variable. F2 went from 7.5 ms to 1.9 ms.

None of these was a design problem. Each was a gap between the design document and the code, and the benchmark found it because it compared every answer before timing it, then printed every loss, not only the wins.

## Where it stands

At 10⁵ statements on an Apple M2 Max, after the fixes:

- **Cyclic patterns:** on the hub-and-spoke graph, a triangle takes 9.7 ms and a tailed triangle 12.6 ms on redb, against 262 ms and 290 ms on SQLite's own plan and 3.4 s on Oxigraph. Our SQLite LFTJ operator gets 7.2 ms and 8.8 ms, so on SQLite the same idea works once it is switched on.
- **Chains and snowflakes:** redb matches or beats SQLite (L1, L3, F1).
- **Lookups:** point and two-hop lookups take 44–51 µs in the now, as-of and valid-at views, slightly faster than SQLite's 62–71 µs and Oxigraph's 87–110 µs.
- **Stars and aggregates with selective constants** are where redb still loses to SQLite: 2.1–3.8× on S1, S3, C1 and C2. Profiles show the time in B-tree seeks; a SQLite covering-index probe in C is simply cheaper than a redb range iterator. Batching probes (sorting the keys of a step so that consecutive seeks move forward) is the next step.
- **Variable-length Cypher paths** are about 3× slower than on SQLite (476 ms against 151 ms for a shortest path) and are the other open item.

At a million statements the cyclic advantage grows: a skewed triangle takes 45 ms on redb, 30 ms on SQLite with LFTJ, 5.1 s on SQLite's nested loops and 57 s on Oxigraph. So does one loss: the snowflake F1 takes 83 ms against 23 ms on SQLite, a probe-heavy plan where every seek counts.

One more finding surprised us. On the dense parts of the skewed graph (diamonds and 4-cycles through the hubs), the generic join is no faster than SQLite's nested loops: all three take 4–7 s. Those queries return millions of matches, and no join order can produce millions of matches faster than it can write them. Worst-case optimality bounds the waste, not the output.

## What we would tell someone building the same

- Keep every permutation you can afford; a key-value store gives you nothing else to plan with.
- Keep exact statistics in the write transaction; estimates from samples go stale, and an estimator that counts rows on every query costs more than the query.
- Push conditions down to the step that binds them, and seed joins from small existence tests.
- Compare every answer with an engine you trust before you time anything, and print the losses.
