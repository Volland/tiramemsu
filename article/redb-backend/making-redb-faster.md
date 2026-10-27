# Why the new engine was not faster, and what made it faster

*Tiramemsu 0.8.0 shipped a second storage engine on redb with its own query runner. It won on cycles and concurrent readers but lost to SQLite on ordinary joins. We asked why, profiled before guessing, and found four problems, none of them in redb. Here is what they were and what the benchmark says now.*

---

When we added redb beside SQLite, the benchmark gave a mixed answer. The redb engine was 113 times faster on a triangle through a hub and three times faster with sixteen concurrent readers. On the WatDiv star and aggregate templates it was 2 to 4 times slower, and at a million statements the snowflake F1 took 83 ms against SQLite's 23 ms.

The obvious question was why a pure-Rust B-tree should be slower than SQLite's. The honest first answer was that it is not much faster, either. Both are B-trees. A point lookup costs about the same on each, and our two-hop lookups took 45 to 65 µs on both engines. The big wins came from things above the storage layer: a worst-case-optimal join for cyclic patterns, and snapshot readers that never wait for a connection pool. Where both engines did the same work, an index nested loop over a selective constant, every probe on our side cost more than it should.

That answer was a theory. We wanted evidence before changing code.

## Measure first

We wrote a small harness that loads the benchmark data once, keeps the files, checks that both engines return the same rows for every query and then times them. A change could be measured in seconds instead of the half hour the full benchmark takes. For profiles we ran one query in a loop under macOS's `sample` and summed the samples per function.

The first theory to fall was ours. We had guessed that the redb file, about a gigabyte at a million statements, no longer fitted redb's 256 MiB page cache. We raised the cache to 2 GiB. F1 took 63 ms both times. The cache was not the problem.

## A byte at a time

The profile of the aggregate query C1 had a surprise at the top: a redb function named `from_bytes` for fixed-size arrays, and under it `u8::from_bytes`, called millions of times.

Our tables stored their values as fixed arrays, `[u8; 24]` for the live permutations and `[u8; 64]` for statements. redb implements its value trait for arrays of any element type, so it decodes `[u8; 24]` the generic way: it builds a vector, decodes each of the 24 bytes as a separate value and converts the vector back into an array. For every row of every scan.

redb also has a zero-copy type, `&[u8; N]`, which returns a reference into the page. Its comment says it uses the same type name as `[u8; N]` so that tables are compatible. So the fix changed the type in our table definitions and nothing else. Files written before read the same, and no format version moved.

## Allocations per probe and per row

With decoding fixed, the allocator stood out. About a third of the samples in C1 were in `malloc` and `free`.

- Every probe built its key prefix in two small vectors and its range end in a third, and redb copied both bounds again.
- Every row a join step produced was a new `Vec<i64>`.
- Hash joins keyed their tables on `Vec<i64>`, hashed with Rust's default SipHash.
- The last step of every join merged the pattern result into an empty starting relation, which copied every row once more for nothing.

None of this is clever to fix. Key prefixes now live on the stack. A join step's rows live in one flat buffer with a fixed number of slots per row, so extending a row writes into the buffer and allocates nothing. A hash join sorts its build rows by a fast hash of the shared variables and checks each candidate as it binds it. Merging with the empty relation returns the other side unchanged.

Together with the decoding fix, C1 went from 4.8 ms to 3.2 ms at 100,000 statements and S1 from 1.5 ms to 0.74 ms.

## The join order was wrong

F1 joins five patterns in a chain: users in one city, what they like, the genre and seller of each product, and where the seller is. At a million statements it was still three times slower than SQLite after the fixes above, and running it on one thread made it worse, 113 ms against 21 ms. That is not a constant factor. It is a different plan.

Explain showed it. Our runner started from `?s locatedIn ?c`, the smallest pattern on its own with 1,052 rows. From there it walked to every product those sellers sell, every like of those products, and only at the end applied the selective pattern, users living in city 1. On the way it built 315,656 intermediate rows. SQLite started from the city and never held more than about 10,000.

The order was greedy: take the cheapest pattern, then repeatedly the connected pattern with the fewest matches per row. Greedy cannot undo its first choice. The fix keeps the greedy completion but tries every pattern as the first one, estimates the intermediate rows of each order from the same exact statistics, and keeps the cheapest. For a basic graph pattern of a few patterns that is a few dozen estimates, nothing next to running the query.

F1 at a million statements went from 83 ms to 20 ms, now slightly faster than SQLite. This problem had nothing to do with redb. It was our planner.

## Filters that never reached the pattern

The Cypher two-hop `MATCH (u {name: 'User 7'})-[:follows]->(f)-[:likes]->(p)` took 47 ms on redb against 14 ms on SQLite. The profile showed almost all of it in evaluating `name = 'User 7'`, once for every `name` statement in the store.

Cypher's property map becomes a pattern under two filters inside an `EXISTS`. Our runner already knew how to push the conditions of a filtered pattern into the join, but only when the pattern sat in a join with others. A pattern alone under filters was scanned in full and filtered afterwards.

Two changes fixed it. A lone filtered pattern now goes through the same join path, so its conditions reach it. And when a condition says a variable equals a constant, the runner binds the variable to the constant before the join, so the pattern seeks on it instead of reading every row. The binding is safe only where it changes nothing: our evaluator compares such a constant by its dictionary id, so binding keeps exactly the rows the condition keeps. Numbers and date-times are excluded, because `7` and `7.0` are equal values with different ids, and a test checks that both still come back.

The Cypher two-hop went from 47 ms to 0.38 ms, 38 times faster than SQLite. The star query went from 134 ms to 95 ms.

## Where it stands

We reran the whole benchmark, all four engines at 100,000 and a million statements, every answer compared across engines before anything was timed.

At a million statements, against the 0.8.0 run:

- **Joins:** F1 83 ms to 20 ms (SQLite 23 ms), C1 35 ms to 21 ms, C2 166 ms to 96 ms, S3 6.0 ms to 3.8 ms.
- **Cypher:** the two-hop 47 ms to 0.38 ms, the star 134 ms to 95 ms.
- **Cycles:** every cyclic query about twice as fast. A triangle through hubs takes 21 ms on redb, 29 ms on SQLite with our leapfrog operator, 4.9 s on SQLite's own plan and 55 s on Oxigraph. redb is now the fastest engine on every cyclic query we run, on the uniform graph as well, where SQLite with the leapfrog operator used to lead.
- **Dense cycles:** the diamonds and 4-cycles of the skewed graph passed the 60-second cap on every engine before. redb now answers the 4-cycle in 38 s and the diamond's checked run within the cap, though its timed runs took 69 s. No other engine answered either, so those two answers were not cross-checked in this run; the differential tests check the same queries on small graphs.

The losses that remain:

- **Stars and aggregates with selective constants** are still 1.3 to 1.8 times slower than SQLite, down from 2.1 to 3.8. Here a SQLite covering-index probe in C really is cheaper than a redb range iterator, and batching probes in key order is the next thing to try.
- **Variable-length Cypher paths** (shortest path and trail) are about 2.5 times slower. The path engine was not part of this work.
- **Size and writes** did not change: a redb file is 3.5 times SQLite's at 100,000 statements and 3.7 times at a million after compaction, and cascades and supersedes are about 40 % slower at a million statements.

One number moved the wrong way and we report it. At 100,000 statements, concurrent readers lost 17 % at four threads and 9 % at sixteen, while at a million statements they gained 20 % and 1 %. Nothing on the path those threads share changed, so we read both as noise between runs. The next run will tell.

## What we would tell someone doing the same

- **Profile before you theorise.** Our first theory, the cache, was wrong, and the real top item, decoding a byte at a time, was not on our list at all.
- **Check what your types cost.** A generic implementation that is fine for an array of strings is not fine for 24 bytes read millions of times. The fix was one type in a table definition.
- **The plan matters more than the store.** The biggest single loss, F1, was a join order, and the biggest single win, the Cypher two-hop, was a filter that never reached its pattern. Neither had anything to do with the B-tree underneath.
- **Compare every answer before you time it.** Every change here was checked against SQLite on the same data, and the binding of constants has a test for the one case where it would be wrong.

The tables are in `bench/backends/README.md`, and the change is `openspec/changes/archive/2026-10-27-tune-native-runner/`.
