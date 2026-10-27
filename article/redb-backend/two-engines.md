# Two engines, one memory

*Tiramemsu now stores a memory in SQLite or in redb, a pure-Rust key-value store, and you choose which when you create the file. Here is why we added a second engine, how one logical database lives in both, and what the measurements say about each, including where the new engine loses.*

---

Tiramemsu is an embedded memory for agents. Every fact is a statement with its own id, two clocks (when we learned it and when it was true) and a history nothing is deleted from. Since the first release that memory has lived in one SQLite file. SQLite gave us a lot for free: a query planner, partial indexes, triggers that guard invariants, FTS5 for text, a WebAssembly build, and twenty years of hardening.

It also gave us limits. Every query we run is generated SQL, so our join strategy is whatever SQLite's planner picks. SQLite plans nested loops over indexes and nothing else. On a cyclic pattern over a skewed graph, a triangle through a hub, that plan can be a hundred times slower than a join built for the shape. We worked around it once already, with a leapfrog-triejoin operator plugged into SQLite as a table-valued function. Readers queue for a small pool of connections. And the file format, the planner and the transaction engine all belong to someone else.

So we built a second engine. You choose it when you create the file:

```sh
tm create memory.redb --backend redb
```

Everything after that, opening, querying, importing, converting, detects the engine from the file's first bytes.

## One logical database

The rule we set before writing any code: **both engines store the same logical database.** The same statement ids, the same transaction numbers, both clocks, the same event log, the same burned ids after a rolled-back speculation. A file converted from one engine to the other answers every query the same way, and `tm convert` replays the whole history, not just the current state.

This rule is what makes a second engine affordable. Behind it is a typed storage trait, `ReadStore` and `WriteStore`: scans by bound positions in a live or historical scope, term and transaction lookups, the event log, volatile values, embeddings and the text index. The transaction engine (asserts, retractions, cascades, supersede, graphs, schema checks, bundles, embeddings) was ported onto the trait once. It runs unchanged on any store that implements it: redb, an in-memory reference store, and, as an experiment we come back to below, SQLite itself.

We trust the port because of an oracle. A property test generates random histories of every operation, including asserts, annotations on statements, supersedes, retractions with cascades, graph membership, schema changes, volatile values, embeddings and speculative transactions. It runs each history on the original SQLite engine and on the trait engine, and compares every report and every final row. It has run 1,000 histories on redb. When we planted bugs on purpose, a reordered cascade or a changed BM25 constant, the oracle failed at once.

## What redb gives us

[redb](https://github.com/cberner/redb) is a copy-on-write B-tree store in safe Rust: ACID transactions, one writer, readers on MVCC snapshots, and crash safety without a write-ahead log. Three properties decided it:

- **Snapshot readers without a pool.** Every view is one read transaction. There is no limit on concurrent views, and a reader never waits for a connection.
- **Ordered byte keys.** We encode every integer big-endian with the sign bit flipped, so byte order is numeric order and one range scan answers one bound prefix.
- **No unsafe code, no C.** The whole storage path is Rust. That matters to us more than a few microseconds.

On redb we keep six live permutations of each statement (`spo`, `sop`, `pso`, `pos`, `osp`, `ops`), so every combination of bound positions is one prefix scan. There are three history permutations for time travel, logs of additions and retractions in transaction order, and exact per-predicate statistics updated in the same write transaction: live statements, distinct subjects, distinct objects. Text search is an FTS5-compatible index in redb tables: the same unicode61 tokenizer with diacritics removed and the same BM25 constants. The oracle checks that text recall scores match SQLite's to 1e-9.

Durability is a knob on both engines. `Normal` is a group commit: a commit becomes durable within 50 ms or 1,000 commits. `Immediate` makes every commit durable when it returns. `None` leaves durability to an explicit `sync`. A child-process crash test kills a writer mid-stream and checks that the file reopens consistent.

## A runner of our own

SQLite brought its query planner; redb brings none. So the redb engine has its own runner, `tm-native`, which evaluates the same logical query plan our SQL generator compiles. SPARQL and Cypher front ends, the planner, the path engine and result decoding are all shared. Only execution differs.

The runner joins acyclic patterns with index nested loops or hash joins, whichever its exact statistics say reads less. It joins cyclic cores with a worst-case-optimal generic join over hash tries, which is the design of Kùzu and Umbra. The companion article, *Worst-case-optimal joins on a key-value store*, goes into how it works and what the benchmarks taught us.

Correctness again comes from comparison, not hope. Every pair in our SPARQL–Cypher differential corpus runs on both engines, as do 32 more SPARQL queries over every fixture and a property test of random queries on random histories. All of them must return the same multiset of rows.

## What we measured

We benchmarked both engines against each other and against [Oxigraph](https://github.com/oxigraph/oxigraph), the reference Rust RDF store on RocksDB. The harness generates a WatDiv-style e-commerce graph and the cyclic graphs of our triangle benchmark. It runs every query once on every engine and compares the results before timing anything; a mismatch stops the run. At 10⁵ statements on an Apple M2 Max, all three engines agreed on every query. Here is what the numbers say, with the full tables in `bench/backends/README.md`.

*Updated on 27 October 2026: these are the numbers after we tuned the native runner, as told in "Why the new engine was not faster, and what made it faster". That article also gives the numbers this release first shipped with.*

**Where redb is stronger than SQLite:**

- Cyclic patterns: a triangle through hubs takes 4.7 ms on redb against 266 ms on SQLite's own plan, 57× faster, and 7.0 ms on SQLite with our LFTJ operator switched on. redb is now the fastest engine on every cyclic query we run, on a uniform graph as well.
- Cypher lookups by property: a two-hop from `{name: 'User 7'}` takes 0.32 ms against 1.9 ms.
- Concurrent readers: 50,400 two-hop lookups per second on 16 threads against 16,100 for SQLite, whose reader pool tops out early.
- Bulk load: 2.9 s against 7.1 s for 100,000 statements.
- Lookups in every view: a two-hop lookup as of an earlier transaction takes 37 µs on redb and 58 µs on SQLite, and point lookups are about 30 % faster in every view.
- The snowflake F1 takes 6.7 ms against 13.9 ms.

**Where redb is weaker than SQLite:**

- Star and aggregate queries with selective constants: WatDiv S1 (0.77 ms against 0.50 ms), C1 (2.0 ms against 1.2 ms) and C2 (7.3 ms against 4.9 ms), 1.4–1.8× slower. Each index probe in redb is a B-tree seek through Rust iterators; SQLite's covering-index probe in C is cheaper, and on probe-heavy plans that adds up.
- Variable-length paths through Cypher: a shortest path takes 365 ms against 147 ms.
- Size: about 1,340 bytes per statement against 384 for SQLite. Six live and three history permutations of fixed-width keys cost space, and compaction only brings it to 1,056.

**Against Oxigraph:** our engines are 1.8–50× faster on the WatDiv templates and 8–740× faster on cyclic patterns, where Oxigraph's evaluator has no worst-case-optimal join. Oxigraph is far ahead on raw writes: an insert takes 5 µs against 110–120 µs for either of our engines. It loads 19–47× faster and stores data 5–17× smaller. On 16 reader threads it also scales better than redb (91,000 against 50,400 lookups per second).

At a million statements the shape holds and the extremes sharpen. A triangle through hubs takes 21 ms on redb, 4.9 s on SQLite's own plan and 55 s on Oxigraph. redb is the fastest of the three for one and for four readers. The snowflake F1, a 3.6× loss when this release first shipped, now takes 20 ms on redb against 23 ms on SQLite. File size still needs care. Right after a bulk load redb holds 2,147 bytes per statement, because it grows its file in large steps while loading, and `tm compact` halves that. So compact after a big import.

## Why we accept the losses

The write gap to Oxigraph is not an accident we plan to close. A tiramemsu write is one ACID transaction that allocates a statement id and a transaction number, checks predicate schemas and cardinality, records the event, keeps the history and cascades retractions to the statements built on top. An Oxigraph insert adds a quad. We prefer the transaction, the history and the invariants over raw speed, and an agent's memory writes far less than it reads.

The same goes for security. Neither engine opens a network port. The redb engine has no unsafe code and no C. Every failure is a typed error, never a silent truncation. Budgets bound every query in time, rows and bytes. A benchmark that ignored these would favour a different system; this one is meant to tell you what you get with ours.

## Which engine to choose

**SQLite** stays the default. It is the right choice when you want the smallest files, the WebAssembly build, FTS5, or the most mature query planner for star-shaped lookups.

**redb** is the right choice when your memory is read by many agents at once, when your queries have cycles (who else is connected to both, which claims support each other), when you load a lot at once, or when you want a storage path with no C in it.

You can change your mind later. `tm convert memory.db memory.redb --backend redb` replays the full history into the other engine, and `tm info` on both files shows the same counts.

## What is next

We tried to move SQLite onto the same trait engine, so the transaction semantics would live in one place. The trait engine on SQLite wrote exactly the same rows as the original engine: the oracle passed, and an old file continued by both engines stayed identical. But it was 70–90 % slower on cardinality replacement and on cascading retractions, because it issues one statement per row where the original issues one set-based statement. Our rule was that the move lands only within 5 % of the old speed, so it did not land, and the work it needs is written down. It was also six times faster at superseding a statement with a hundred annotations, which tells us the original engine has a slow path worth fixing on its own.

That is the shape of this whole change: two engines, one logical memory, and numbers that say plainly where each one wins.
