# Datalog over time: rules, constraints and derived beliefs in an agent's memory

*An agent's memory is not only what it was told. It is also what follows from what it was told, what must never be true, and what stops being true when a premise is withdrawn. Here is how Tiramemsu's Datalog engine handles all three over valid time, what it measured against N3 and Soufflé, and what is not built yet.*

---

Agent memory systems usually stop at storage and recall. You write facts, you search them. But a memory that is going to be trusted needs three more things, and each one is a different kind of rule.

- **Inference.** Alice works at Acme and Acme is in Berlin, so Alice works in Berlin. Nobody wrote that last fact. It follows.
- **Prohibition.** There must never be two CEOs at once. A write that would create them should fail, not be noticed a month later.
- **Retraction.** If the fact "Bob manages Carol" turns out to be wrong, everything that was concluded from it must go too. Nothing should keep standing on a premise that is gone.

All three have a time dimension that is easy to get wrong. "Two CEOs" is a violation only if their terms overlap. "Alice works in Berlin" holds only while both of its premises hold. A rule over facts that were true in different years must not combine them.

Tiramemsu is an embedded bitemporal store where every fact is a statement with its own id, a *valid time* (when it was true in the world) and a *transaction time* (when the store learned it). It has two rule languages: Notation3 (N3), run by the eyeron reasoner, and Datalog, run by its own engine, `tm-datalog`. This article is about the second one: what it computes, how it reads time, and how its conclusions behave as memory. It ends with a comparison to N3 in the same store, including the numbers where Datalog is faster and where it is not.

Every example below is taken from, or checked against, a test in `crates/tiramemsu/tests/`, which runs each scenario on both storage engines (SQLite and redb) and requires the same answer from both. In the examples, `v:` is the built-in prefix `urn:tiramemsu:v:`, and `rdf:type` is the usual one.

## Datalog in one paragraph

A Datalog program is a set of rules, `head :- body.`, over relations. The engine finds every fact the rules can derive, applying them until nothing new appears. Tiramemsu's engine is bottom-up and semi-naive: each round only joins against what was new in the last round. Programs are written as Soufflé-style text, the one form that is stored, parsed and passed through every binding:

```
v:reportsTo(X, Y) :- v:manager(X, Y).
v:reportsTo(X, Z) :- v:reportsTo(X, Y), v:manager(Y, Z).
```

An atom like `v:manager(X, Y)` reads the stored statements whose predicate is `v:manager`, with subject `X` and object `Y`. `v:manager(X, Y) @ E` also binds the statement's id to `E`. A program can also contain `?-` goals and `constraint name warn|error :- body.` lines, which we get to below.

A goal that binds constants, such as "who does Alice report to", is not answered by computing the whole closure and filtering. The engine rewrites the program with demand (magic-set) predicates so that only the relevant part is derived, and falls back to the full program for a stratum where the rewrite cannot apply. The `explain` output names the fallback.

## Time is part of the meaning

The central design choice, decision DL5 in `lat.md/datalog.md`, is that a conclusion holds at an instant exactly when it follows from the facts valid at that instant.

Take Alice working at Acme from 2020 to 2025 and Acme located in Berlin from 2024:

```
v:worksIn(P, L) :- v:worksAt(P, C), v:locatedIn(C, L).
```

The test `overlap_gives_the_intersection` asserts that the only conclusion is `worksIn` valid from 2024 to 2025: the intersection. If Alice's job ended in 2022 and Acme only moved to Berlin in 2024, the test `facts_never_simultaneously_valid_do_not_combine` asserts there is no conclusion at all. Two facts that were never true together never meet in one rule body.

To make that cheap, every tuple in the engine carries a *coalesced interval set* instead of a single interval. A join intersects the sets of its premises. A union, such as two rules deriving the same fact, merges them. The test `two_derivations_coalesce` has Alice at Acme until 2023 and at Beta from 2023, both in Berlin; the engine returns one `worksIn` conclusion, 2020 to 2026, not two touching ones.

The timeline is the stored one: integer epoch milliseconds, half-open intervals, with symbolic open ends. A fact valid "from now on" has an unbounded end, and the arithmetic handles that explicitly.

This is also why one pass is enough. An alternative is to cut the timeline into segments at every bound of every fact, run the rules once per segment and merge equal neighbours; the N3 path does that. Carrying interval sets through joins means the engine sees each derivation once and the cost follows the intervals involved, not the number of segments in the window.

### DatalogMTL: operators over the past and future

DatalogMTL extends Datalog with metric temporal operators in rule bodies. Tiramemsu supports these: `past[a,b]` and `future[a,b]` (sometime within the offset range), `always_past[a,b]` and `always_future[a,b]` (throughout it), and the binary `A since[a,b] B` and `A until[a,b] B`. The always operators may also appear in heads. Three tests show the flavour.

*Sometime in the past.* A patient has a fever on day 10. A rule marks them as alerting for three days after:

```
alert(X) :- past[0d, 3d] v:fever(X, true).
```

The test `fever_within_the_last_three_days` asserts that the fever, valid from day 10 to day 11, makes `alert` valid from day 10 to day 14: the fever interval, stretched forward by the window.

*Always in the past.* A person employed for the whole of days 0 to 100:

```
tenured(X) :- always_past[0d, 30d] v:employed(X, true).
```

The test `always_in_the_past` asserts `tenured` is valid from day 30 to day 100. It only becomes true once thirty days of unbroken employment are behind it.

*Since.* An alarm raised at time 5 and an unacknowledged flag from 5 to 20:

```
pending(S) :- v:unacked(S, true) since[0, inf] v:alarm(S, true).
```

The test `since` asserts `pending` is valid from 5 to 20: it holds as long as `unacked` has held continuously since the alarm.

*Always in a head.* A booking valid from day 1 to day 2 blocks the resource for a further week:

```
always_future[0d, 7d] blocked(R) :- v:booked(R, true).
```

The test `holds_for_the_next_week` asserts `blocked` is valid from day 1 to day 9. A `future` or `past` operator (sometime) in a head is refused with `DatalogNotHorn`, since it would not be Horn: it would say "at some unspecified point", and the engine would have to pick one.

### Recursion through time needs a window

Operators make a classic problem. A contagion rule says that someone is contagious if they had contact with a contagious person within the last three days:

```
contagious(X) :- v:sick(X, true).
contagious(Y) :- past[0d, 3d] contagious(X), v:contact(X, Y).
```

Left alone, a rule like this can keep pushing the end of an interval forward forever, producing ever longer intervals. The engine analyses the program, and if it finds a dependency cycle through a metric operator it refuses to run without an explicit window: the test `contagion_needs_a_window` asserts `DatalogUnboundedRecursion`. With `DatalogOptions::window` set to days 0 to 30, `contagion_within_a_window` finds the chain `a`, `b`, `c` across contacts in days 2 to 4, with every interval clipped to the window. Within a window the coalesced fixpoint is finite. A separate fuse on iterations and interval fragments reports `Incomplete` rather than running away, as the N3 limits do.

### Allen's relations

Metric operators say how far apart things are. Allen's interval algebra says how two intervals relate: before, meets, overlaps, starts, during, finishes, equals and the six converses, thirteen in all. They are builtins that take two statement ids (or two intervals), and share their definition with SPARQL and Cypher through `Valid::relate` in `tm-core`.

There is a trap, found while writing the tests. The atom `v:job(a, x) @ E1` is a *fact*: it holds over the statement's valid time. Under pointwise semantics two facts that were never valid together never meet in one body, so "this job meets the next job" cannot be written over facts. The statement's *record* holds always, and `stmt(E, S, P, O)` reads it. The test `job_meets_the_next_job` stores a job from 2020 to 2022 and another from 2022 to 2025 and asks the question over records:

```
stmt(E1, v:a, v:job, v:x), stmt(E2, v:a, v:job, v:y), meets(E1, E2)
```

One row, and `overlaps(E1, E2)` over the same records gives none. The same goal through the facts, `v:job(v:a, v:x) @ E1, v:job(v:a, v:y) @ E2, meets(E1, E2)`, has no row, because the two jobs are never valid at one instant. `valid`, `added` and `tx_instant` are records too, which is how transaction time reaches a program as data.

## Negation and aggregates, stratified

Rules about absence are where memory systems usually get vague. "Employees with no manager" is `not`:

```
unmanaged(X) :- rdf:type(X, v:Employee), not v:manager(X, _).
```

Negation is stratified: the engine computes `v:manager` completely before it evaluates anything that negates it, and a program that cannot be stratified is refused with the offending cycle named (`negative_cycle_refused`). It is also pointwise in time. `not p(X)`, for a binding that a positive atom also supplies, is the complement of `p`'s interval set. The test `unmanaged_until_a_manager_arrives` has Bob as an employee and a `manager` statement valid from 2023. The answer is `unmanaged(bob)` valid *until* 2023, an interval that ends the moment the manager starts.

Aggregates work the same way. The test `headcount_changes_with_hires` stores Acme employing `a` from 2020 and `b` from 2022 to 2024, and runs:

```
headcount(C, N) :- rdf:type(C, v:Company), N = count : { v:employs(C, _) }.
```

The result is two rows: `N = 1` valid from 2020 to 2022 and again from 2024; `N = 2` valid from 2022 to 2024. The aggregate sweeps the interval endpoints of its group, so a count over a window of years costs by the number of endpoints, not the number of days. No stratum is re-run per global segment.

Two more details worth knowing. A predicate IRI that a rule head defines also holds the stored statements of that predicate (decision DL26), so stored and derived facts combine as they do in N3; that means a rule that negates its own head predicate is recursion through negation and is refused. And a head may invent a node with `exists`, giving a rule such as "every employee without a manager gets an assigned manager":

```
exists M: v:assignedManager(X, M) :- rdf:type(X, v:Employee), not v:manager(X, _).
```

The invented node is a skolem node found or created deterministically from the rule and its bindings, so re-running reuses it (`stable_invented_manager`). Recursion through invention needs `limits.maxNewValues`, or the program fails with `DatalogUnboundedInvention`.

## Constraints enforced at commit

A rule that derives is useful. A rule that refuses is a different tool, and a Datalog body is a good way to state it: a constraint is a body that must have no match.

```
constraint one_ceo error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y.
```

There are two severities. A `warn` constraint is check-only: `View::check` returns its violations with their bindings, the statement ids involved and the intervals on which they hold, at any as-of or valid-at view. An `error` constraint can additionally be *enforced*. The program is stored as a node with a `dl:source` statement, and `TxDatalog::enforce` writes `(program dl:enforced true)`. From then on, every commit evaluates the program's `error` constraints before it lands, and a violation rolls the transaction back with `Error::ConstraintViolated`, carrying the constraint name and the violations.

Because the check is temporal, it distinguishes a conflict from a succession. The test `two_ceos_at_once` has `a` as CEO from 2020 to 2024 and `b` from 2023. It reports a violation on exactly 2023 to 2024, with both statement ids and both variable orderings. The test `successive_ceos_are_fine` has `b` start in 2024, and nothing is reported. The test `second_ceo_refused` enforces the constraint against an existing CEO and then tries to commit a second one valid from 2025: the commit fails with `ConstraintViolated` for `one_ceo`, each violation valid from 2025, and the live statement count is unchanged. The test `successor_accepted` shows the legitimate way: a single transaction closes the first CEO's valid time at 2025 with `supersede` and asserts the second one from 2025, and it commits.

The check can react to removals, because negation does. With the constraint `owned error :- rdf:type(P, v:Project), not v:owner(P, _)`, retracting the project's only owner statement fails the commit (`negation_reacts_to_retraction`). It also covers metric windows: `cooldown error :- v:dose(P, _), past[1ms, 6h] v:dose(P, _)` refuses a second dose six hours or less after the first, and accepts one eight hours later (`metric_window`).

Enforcement has to be cheap, because it runs inside the single writer on every commit. Three design choices keep it so:

- **Delta-seeded.** The check starts from the statements the transaction added and retracted, not from the whole store: each positive stored atom gets a variant that reads only the added rows that fit its constants, and each negated atom a variant that starts from the retracted rows. A violation counts only if the view as of the previous transaction did not already hold it over the same instants. The test `unrelated_write_skips_the_check` asserts that a write no constraint atom fits runs no check.
- **Budgeted and closed on failure.** A `ConstraintBudget` (`timeout_ms` and `max_seed_rows`, set as `constraint_budget` in the options) bounds the work, and over budget the transaction fails with `ConstraintBudgetExceeded` instead of letting an unchecked write through (`budget_fails_closed`).
- **Engine independent.** The check reads uncommitted state through the storage traits, so it behaves the same on SQLite and redb.

Registering an enforced program over data that already violates it is itself refused, with the violations (`enforce_over_violating_data`), as schema changes are. A dry run lists the violation instead of committing (`dry_run_lists_the_violation`).

Limits are explicit. An `error` constraint may read stored atoms, inlined non-recursive rules and metric operators. One over recursive rules is check-only in this version (`recursive_constraint_not_enforceable`: enforcing it fails with `ConstraintNotEnforceable`), and aggregates, `since`/`until` and operators with an infinite window make a constraint not enforceable. A `warn` constraint may use them freely.

The cost, measured by `cargo run --release -p tm-bench --features datalog --example datalog_constraints -- --size 1000000 --reps 5` on 2026-10-28: a 100-statement commit on a store of one million `v:role` statements. Best of five, in milliseconds:

- No enforcement: 3.78 on SQLite, 2.45 on redb.
- Enforced, no row fits the constraint's atoms (so nothing is checked): 1.78 on SQLite, 2.55 on redb.
- Enforced, every row seeds a check (`banned error :- v:role(X, R), v:banned(R, true)`): 3.10 on SQLite, 2.81 on redb.

On redb that last row is 15 % over the baseline, inside the proposed gate of 20 %; on SQLite the difference is within noise. Before the checks were made to plan without the store's statistics and never load a predicate, each check counted up to 4,096 rows per atom through SQLite and the seeded commit took 76 ms.

## Programs that materialize: derived beliefs with support

Queries and checks leave nothing behind. *Materialization* writes a program's conclusions into the store as statements, and the point is what kind of statements they are.

`Db::materialize_program` evaluates a stored program on a snapshot and writes the conclusions of its IRI relations as **derived beliefs**: ordinary statements, each with a *support*, the statements it was derived from. The support of a conclusion is its premises plus the statement holding the program's text. This reuses the support cascade that Tiramemsu has for any derived statement: retract a premise and everything that rested on it is retracted too, in the same transaction.

Using the reporting chain of the tests, with `alice manager bob` and `bob manager carol` stored, and the program `REACH` from the start of this article:

```rust
store(db, "reporting", REACH);      // tx.store_program(v("reporting"), text)
let r = db.materialize_program(&v("reporting"), &MaterializeProgramOptions::default())?;
```

The test `materialized_chain` asserts three derived statements: `alice reportsTo bob`, `bob reportsTo carol` and `alice reportsTo carol`. The last one's support has three members: the earlier derived `alice reportsTo bob`, the stored statement `bob manager carol`, and the program statement. Each conclusion also carries a `dl:derivedBy` label.

Two things then follow, and both are tested.

**Conclusions fall with premises.** In `retracting_a_premise`, retracting `bob manager carol` retracts, through the cascade, both the derived statements that rested on it. One live `reportsTo` remains (`alice reportsTo bob`), and the report shows at least two retractions with the kind `Cascade`.

**Conclusions fall with the program.** In `editing_the_program` the program is replaced by a version that only has the first rule. Editing is a supersede, which retracts the old program statement, and everything derived with it goes at once: no live `reportsTo` remains until the program is run again, after which there are two. The rules are part of the grounds of what they conclude.

Re-running is cheap and honest. `re_running_is_a_no_op` asserts that a second run finds all three conclusions already existing, derives none and writes exactly one statement: the transaction's `dl:materialized` label. A `dry_run` computes the same report and writes nothing.

### Valid time on the way out

A conclusion is written with the valid time it holds over. In `temporal_conclusion`, a fever stored for day 10 to 11 and the rule `v:alert(X, true) :- past[0d, 3d] v:fever(X, true).` materialize into one `alert` statement valid from day 10 to 14, whose support is the fever statement and the program. Where one derivation holds on several disjoint intervals, each interval becomes its own statement with its own support, so each falls with the premises of that interval alone.

When a later run finds the same derivation over a longer interval, the stored statement is *widened* by supersede to the hull; intervals grow, they never shrink. In `widening_across_windows` a first run is restricted to a window of 2020 to 2022 and writes `worksIn` for those years; the next unrestricted run reports one widened statement and the live interval becomes 2020 to 2024. With `require_complete`, a run that hit a limit refuses to commit at all (`require_complete`: `DatalogIncomplete`, nothing written).

### Reconciliation: what monotonic reasoning cannot do

All of the above works because the programs are *monotonic*: adding facts only adds conclusions, so the only way a conclusion becomes wrong is by a premise being retracted, which the cascade already handles.

Negation and aggregates break that. If `bob` is unmanaged and a `manager` statement arrives, the conclusion `unmanaged(bob)` becomes false, but nothing was retracted. The cascade, which reacts only to retractions, never fires.

For these strata the engine *reconciles*: it diffs the new run against the program's live conclusions, and retracts those no longer derived. In `unmanaged_becomes_managed`:

```
v:unmanaged(X, true) :- rdf:type(X, v:Employee), not v:manager(X, _).
```

Bob is an employee and the first run derives `unmanaged(bob)`. Then `bob manager carol` is asserted, and the second run reports one reconciled conclusion; no live `unmanaged` remains. The reconciliation is an ordinary, explicit retraction, so it is kind 0 in the retraction log and the cascade runs from it; there is no new retraction kind that would lock older binaries out of newer files. The materialize transaction carries `dl:reconciled` with the program and `dl:reconciledCount`. The aggregate version is `aggregates_track_headcount`: after the first run `headcount` is 1, after a second employee is added and the program re-run, it is 2, and only the 2 is live.

The honest catch is in the report. A negated or aggregated stratum is correct *at the moment of the run*, and a write afterwards can make it stale before the next run. The report lists such strata in `needs_rerun_after_writes`, with the predicates they conclude. A monotonic program has that list empty and is never reconciled (`monotonic_strata_are_not_reconciled`).

### Snapshot, then commit

Materialization evaluates on a read view *outside* the writer and takes the single-writer lock only for a short commit. Between the two a premise may be retracted. The commit core skips every conclusion whose support is no longer live and reports it in `stale_support`, so nothing is written whose grounds are gone. In `premise_retracted_during_evaluation`, the edge under Alice's chain is retracted between evaluation and commit; her conclusions are skipped and Dan's, unrelated, is written. Missed conclusions come back on the next run.

### What is not built: incremental runs

Every call evaluates the whole program on a snapshot and writes only the difference. **Incremental materialization is not built.** The design (decision DL22 in `lat.md/datalog.md`) is DRed-style: seed the next run from the statements added since the last labelled run, and re-derive only what the cascade retracted since. The label `dl:materialized` is written so that such a run has its base, but the engine today keeps no state between runs and always recomputes. For a large store with small changes, that is the next piece of work, not something this article can claim.

## How it compares with N3 in the same store

Tiramemsu keeps both languages, because they are different instruments. They share the machinery that matters for memory: a stored program is a node whose text is a statement (`n3:source` for rule sets, `dl:source` for Datalog programs), editing is a supersede, `as_of` runs use the program believed at that time, and materialized conclusions are derived beliefs with support that includes the program statement. The commit core was extracted into one module that both use, and N3's materialization tests ran unchanged afterwards. A node cannot hold both: mixing them fails with `RuleLanguageConflict`.

Where they differ:

- **Evaluation.** N3 runs through eyeron, a general N3 reasoner, and the valid-time behaviour is a sweep over the segments between bounds. It keeps one proof per conclusion and segment. The Datalog engine is set-at-a-time, with interval sets through the joins, and sees every derivation of a stratum. On the same Horn rules the two give the same answers: the benchmark requires N3 to agree with Datalog on the closure whenever N3 completes.
- **Non-monotonic rules.** `Db::materialize` refuses rule sets using negation as failure, aggregates, the clock or `list:map`, because the cascade would not notice a conclusion made stale by an added fact. Reconciliation by diffing is, in the N3 notes, still an open follow-up. The Datalog path has it. `View::reason` in N3 accepts these programs for read-only use.
- **Expressiveness.** N3 keeps quoted formulas, N3 built-ins and optional proof text. Datalog has DatalogMTL operators, Allen relations, stratified aggregates and goal-directed evaluation with magic sets, and its constraints are a first-class, enforced thing. If you want formulas as terms, you want N3; if you want closure queries, temporal windows or an integrity constraint, you want Datalog.
- **Speed.** Measured on 2026-10-28 on an Apple M2 Max, `cargo run --release -p tm-bench --features datalog,n3 --example datalog -- --noise 100000 --reps 2`, with 100,000 irrelevant `v:name` literals in every store. Before any time is printed, SQLite and redb must return the same facts and, where applicable, Soufflé and N3 must agree on the closure.

- Transitive closure on a hub graph (2000x200, IRI form, 422,100 facts): Datalog `reason` on SQLite 2,594.7 ms; N3 (eyeron) on SQLite 17,175.2 ms.
- Transitive closure on a small-world graph (1000 nodes, IRI form, about a million facts): Datalog `reason` on SQLite 7,617.3 ms; N3 on SQLite 491,073.9 ms, and incomplete (999,221 facts).

On the hub closure Datalog is 6.6 times faster. On the small world N3 gave up incomplete after 491 seconds; Datalog finished in 7.6. A DatalogMTL contagion run over 2,000 timed contacts takes 3.6 ms on redb and 6.5 ms on SQLite, in one pass.

The numbers also contain the engine's own losses. The proposed gates were at least ten times faster than N3 and within twice the time of Soufflé's interpreter on closure. Neither is met: the hub closure is 6.6 times, not ten; and Soufflé's interpreter, including its process start and fact loading, is faster on the plain closures than Tiramemsu's native Datalog (449 ms against 1,694 ms on SQLite and 1,586 ms on redb for a 1,500-node chain; 157 ms against 682 and 651 on the hub; 482 ms against 1,486 and 2,109 on the small world). The ratios are 3.1 to 4.3 times slower. The README of the benchmark attributes it to tuple-at-a-time insertion and interval-set handling in the semi-naive loop; the worst-case-optimal join for cyclic bodies is not built yet either. Tuning is open.

Where it already wins is the goal with constants. A demand-driven goal runs in 0.4 to 12 ms (for example, 2.7 ms on redb for the 1,500-node chain, against 1,586 ms for the full closure) because the magic-set rewrite derives only what the goal needs.

Not measured yet, and so not claimed: the RDFS and OWL 2 RL subset on WatDiv, a temporal LUBM-style set, runs at a million statements, and comparisons with Nemo and MeTeoR.

## Using it

Datalog sits behind the facade feature `datalog`, off by default. The Rust calls are `View::datalog`, `View::datalog_reason` (conclusions shaped like N3's), `View::check`, `View::program_info`, `View::explain_datalog`, `TxDatalog::store_program`, `TxDatalog::enforce` and `Db::materialize_program`. The JSON bridge has the matching ops (`datalog`, `datalogReason`, `check`, `programInfo`, `explainDatalog`, `storeProgram`, `enforce`, `materializeProgram`), so Node, Python and the browser build have them, the MCP server offers `datalog_query`, `check_constraints`, `store_program`, `enforce_program` and `materialize_program`, and the CLI has `tm datalog`, `tm check`, `tm program put|show|enforce|unenforce` and `tm materialize-program`. SPARQL and Cypher only read what was materialized; a `SERVICE` and `CALL` for single goals are the next change, `add-datalog-query-calls`, and are not part of what is described here.

## What this adds up to

For an agent's memory, the useful property is not that Datalog is a faster way to run rules. It is that the three kinds of rule answer to the same two clocks and the same history as everything else:

- an inference has a valid time, computed from its premises, and a transaction time, the instant it was written;
- a prohibition looks at intervals, so a succession is not a conflict and an overlap is, and it blocks the write that would break it;
- a derived belief carries its grounds, so a retraction or a program edit removes what it supported, and a negated or aggregated one is reconciled instead of left standing.

What is missing is stated above: incremental runs, a worst-case-optimal join for cyclic bodies, and the speed of Soufflé's interpreter on plain closures. The design notes are in `lat.md/datalog.md`, the requirements in the OpenSpec specs `datalog-language`, `datalog-evaluation`, `datalog-temporal`, `datalog-programs`, `datalog-constraints` and `datalog-materialization`, and the benchmark in `bench/datalog/README.md`.

*Related: [Reasoning with N3 in Tiramemsu](https://volland.github.io/tiramemsu/n3/), the mini book on the other rule language.*
