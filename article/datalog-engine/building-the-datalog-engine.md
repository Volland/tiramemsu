# Building a Datalog engine for a bitemporal store: what we chose, what broke, what the numbers say

*Tiramemsu already had an N3 reasoner. It added a second rule language, Datalog, in its own crate, in four steps over one day of work. This is the engineering account: why it was not an embedded engine, how time went into the join, the bugs the tests found, and two performance gates that were missed.*

---

The companion article, [Datalog over time](datalog-temporal-reasoning.html), explains what the engine does. This one is about how it was built and what the building taught, written from the design record in `lat.md/datalog.md` and from the tests and benchmark in the repository.

## Why not embed an engine

The obvious route is to depend on a Datalog engine that already exists. Several are fast and mature. The decision, recorded as DL2, was to write one, and the reasons are specific to this store rather than a judgement of the others.

Rule sets in Tiramemsu are data. A program is a statement, edited by supersede, read as of a past instant, versioned like any fact. An engine whose rules are fixed at compile time cannot be that. An engine with its own storage would copy the scope of a view into itself on every run, and would know nothing about the two clocks. Valid time is the part that matters most: a fact holds at an instant exactly when it follows from the facts valid at that instant, and an off-the-shelf engine treats time as just another column unless you build the semantics on top of it.

There was also a packaging constraint. The N3 integration cannot be published to crates.io because its reasoner is a git dependency. A new engine with registry dependencies only, `tm-core`, `tm-store`, `rustc-hash` and `smallvec`, publishes as a normal crate. It did: `tm-datalog` 0.10.0 is on crates.io.

None of this is a claim that writing it was cheaper. It is a claim that the properties above, rules as memory, time in the join and a publishable crate, were the requirements, and none of the options weighed in the design interview provided all three. I did not benchmark the alternatives against this engine.

## Time inside the join

The central design problem is valid time. The usual way to get "this conclusion holds when its premises do" is to cut the timeline into segments at every bound, run the rules once per segment and merge equal neighbours. It is correct and simple, and its cost grows with the number of distinct bounds in the data, which in a memory with a lot of churn is large.

The engine does something different. Every tuple of every relation carries a coalesced set of half-open intervals of epoch milliseconds. A join intersects the sets of its premises. Two rules deriving the same fact merge their sets. Negation subtracts. An aggregate sweeps the endpoints of its group. One pass answers for the whole timeline.

The metric operators of DatalogMTL are then functions on those sets. "Held at some instant within the last three days" stretches each interval forward by three days. "Held throughout" shrinks it. `since` and `until` combine two sets with a sweep. The edge cases are the ones you would expect: open ends must stay open under arithmetic, and a window of `inf` must not overflow.

How do you trust that? Two ways, both in the tests. The operators have unit tests that compare each one against a brute-force definition evaluated instant by instant over a range of small sets and windows, so a mistake in an endpoint shows up as a specific instant that disagrees. And a larger test, `instant_by_instant_oracle`, runs random programs with positive rules, negation and an aggregate over random facts with random intervals, and checks that at every probed instant the interval engine's answer equals what an atemporal run over the statements valid at that instant derives. It passed on its first run. That says the interval algebra was right; it is not a reason the oracle would have been unnecessary, because without it nothing would have said so.

## Four things the tests found

**A time model that hides relations between statements.** The first version of the Allen-relation test was the obvious one: two jobs, one valid 2020 to 2022 and one 2022 to 2025, and `meets` should hold. It did not, and it should not. Under the pointwise semantics, the atom `v:job(a, x) @ E` is a fact that holds only while the statement is valid. Two statements that were never valid together never meet in one rule body, so there was nothing to relate. The fix was a distinction in the language, not in the engine: `stmt(E, S, P, O)` is the statement's record, which holds at every instant, and the relation builtins run over records. Facts and records are different things, and the spec now says so (DL25).

**A rule that negates its own head.** The program `assignedManager(X, M) :- employee(X), not manager(X, _)` looks harmless until the head predicate is also stored data. Because a predicate IRI that a rule head defines also holds the stored statements of that predicate (DL26, matching N3), a rule that negates its own head predicate is recursion through negation, and stratification rightly refuses it. The test that exposed it was written with `manager` as both the stored and the invented predicate. The fix was to the example: derive a differently named predicate.

**Counting the store to plan the join.** Constraints enforced at commit run inside the single writer, so their cost matters. The first version planned each seeded check the way a query is planned, by counting statements per atom through the store. On a million statements that took 76 milliseconds for a hundred-statement commit on SQLite. A check that starts from a handful of changed rows does not need statistics: the changed rows are the small side by construction. Planning without reading the store, and never loading a predicate into memory, brought it to 3 milliseconds.

**A clock that panics on wasm.** The commit hook gave every check a deadline with `std::time::Instant`, and built it on every commit, even when nothing was enforced. On `wasm32-unknown-unknown`, `std::time` is not supported, so any write through the browser build panicked. It passed every native test. The wasm CI job caught it, and I reproduced it locally under Node before fixing it: the hook now returns before touching a clock when nothing is enforced, and runs without a deadline on wasm.

## Constraints that cost what the write costs

An enforced constraint is a Datalog body that must have no match. Checking all of it on every commit would grow with the store. The check is seeded instead: each positive stored atom gets a variant that reads only the rows the transaction added that fit the atom's constants, and each negated atom gets a variant that starts from the rows it retracted. A violation is kept only if the view as of the previous transaction did not already hold it over the same instants, so an old violation does not fail an unrelated write.

The property test `seeded_check_equals_full_check` is what makes this believable: on random histories with random constraints, a transaction is refused by the seeded check exactly when a full check after it finds an error violation that a full check before it did not. It asserts that refusals actually happen, since a test where nothing is ever refused proves nothing.

The measured cost, on a store of a million statements and a hundred-statement commit: on redb, 2.45 milliseconds without enforcement and 2.81 with a constraint every row has to be checked against, about 15 percent more, inside the 20 percent gate set beforehand. On SQLite the difference was within noise.

## The numbers, including the misses

Gates were written down before the benchmark ran, and two were missed.

The first was to be at least ten times faster than N3 on the same programs. On a hub-shaped graph with 422,100 closure facts, the Datalog engine's `reason` took 2.6 seconds and N3 took 17.2: 6.6 times, not ten. On a small-world graph of about a million facts, N3 gave up incomplete after 491 seconds with 999,221 facts; Datalog finished in 7.6 seconds. So the gate is missed on the case where N3 completes, and the gap is much larger where it does not.

The second was to be within twice the time of Soufflé's interpreter on closures. It is 3.1 to 4.3 times slower, including Soufflé's process start. The engine joins by hash indexes and nested loops over tuples held one at a time; the design called for sorted columns and a worst-case-optimal join for cyclic bodies, which were not built, and the missing join is recorded as debt on the relation type in the code.

What does win is the shape agent memory mostly asks. A goal with constants is rewritten with demand predicates, so only what the goal needs is derived: 2.7 milliseconds on redb for a 1,500-node chain, against 1.6 seconds for the whole closure. A temporal contagion rule over 2,000 timed contacts takes 3.6 milliseconds on redb in one pass. Not measured, and so not claimed: RDFS and OWL rule sets on WatDiv, a temporal LUBM-style workload, runs at a million statements, and comparisons with Nemo and MeTeoR.

## What was built, and in what order

The work was four OpenSpec changes: the engine, then enforced constraints, then materialization of programs as derived beliefs, then goals callable from SPARQL and Cypher. Each has a specification with scenarios, and each scenario has a test that names it. The scenario tests run on SQLite and redb and require identical results, more than ninety of them in total. The materialization step extracted the commit logic shared with N3, so that both rule languages write derived beliefs the same way, and N3's own tests ran unchanged against the refactor.

The traceability checks tie each requirement to the code that implements it and the test that proves it, which is what let the work be verified step by step, and what flagged the two tasks that were left undone.

## What is still missing

Incremental materialization is not built: every run evaluates the whole program, writing only the difference. The worst-case-optimal join is not built. A goal called from SPARQL or Cypher cannot return a value the store has no id for, such as a string the rules computed. The randomised comparison of the three call surfaces, the Rust call, SPARQL and Cypher, is one fixed goal on both engines rather than a property test. The facade crate that exposes all of this is not on crates.io, for the dependency reason in the N3 article.

The lesson I would take is about order. The oracle tests, the brute-force evaluator for the operators and the instant-by-instant comparison, were written before the engine was trusted, and they are why the time semantics could be changed with confidence. The performance gaps are the other half of the lesson: gates written down in advance are uncomfortable exactly when they are useful.

*Related: [Datalog over time](datalog-temporal-reasoning.html), what the engine does; [Putting an N3 reasoner inside a bitemporal memory](n3-reasoner-integration.html); the [Datalog guide](../datalog/).*
