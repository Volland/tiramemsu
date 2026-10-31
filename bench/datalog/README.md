# Datalog benchmark

`cargo run --release -p tm-bench --features datalog,n3 --example datalog -- --noise 100000 --reps 2`
(`crates/tm-bench/examples/datalog.rs`, OpenSpec change `add-datalog-engine`, task 9).
Every store holds the graph's `v:e` edges plus 100 000 `v:name` literals that no rule
reads. Before a time is printed, SQLite and redb must return the same facts, Soufflé
(when `souffle` is on the PATH) must agree on the closure, and N3 must agree on the
IRI form of the closure when it completes. Times are the best of the repetitions, in
milliseconds; Soufflé's include its process start and fact loading.

Measured on 2026-10-29 after `tune-datalog-performance`, Apple M2 Max, Soufflé 2.x
interpreter, `--noise 100000 --reps 2`. The "evaluation only" and "results only" rows
come from explain's phase times (best of the repetitions): evaluation is the engine,
results is decoding, ranking and sorting the 10⁶ rows.

| workload | engine | ms | facts |
|---|---|---:|---:|
| tc chain-1500 | datalog sqlite | 644.6 | 1124250 |
| tc chain-1500 | datalog sqlite evaluation only | 424.0 | |
| tc chain-1500 | datalog sqlite results only | 224.4 | |
| goal chain-1500 | datalog sqlite (demand) | 12.0 | 1498 |
| sg chain-1500 | datalog sqlite | 10.9 | 1499 |
| tc chain-1500 | datalog redb | 639.3 | 1124250 |
| tc chain-1500 | datalog redb evaluation only | 402.5 | |
| tc chain-1500 | datalog redb results only | 219.6 | |
| goal chain-1500 | datalog redb (demand) | 2.4 | 1498 |
| sg chain-1500 | datalog redb | 1.7 | 1499 |
| tc chain-1500 | souffle interpreter (incl. start) | 610.5 | 1124250 |
| tc hub-2000x200 | datalog sqlite | 221.6 | 422100 |
| tc hub-2000x200 | datalog sqlite evaluation only | 140.1 | |
| tc hub-2000x200 | datalog sqlite results only | 79.5 | |
| goal hub-2000x200 | datalog sqlite (demand) | 7.8 | 201 |
| tc hub-2000x200 (IRI) | datalog reason sqlite | 233.8 | 422100 |
| tc hub-2000x200 (IRI) | n3 (eyeron) sqlite | 19654.9 | 422100 |
| tc hub-2000x200 | datalog redb | 223.1 | 422100 |
| tc hub-2000x200 | datalog redb evaluation only | 138.4 | |
| tc hub-2000x200 | datalog redb results only | 79.7 | |
| goal hub-2000x200 | datalog redb (demand) | 0.4 | 201 |
| tc hub-2000x200 | souffle interpreter (incl. start) | 163.4 | 422100 |
| tc smallworld-1000 | datalog sqlite | 642.5 | 1000000 |
| tc smallworld-1000 | datalog sqlite evaluation only | 441.6 | |
| tc smallworld-1000 | datalog sqlite results only | 198.6 | |
| goal smallworld-1000 | datalog sqlite (demand) | 10.6 | 1000 |
| tc smallworld-1000 (IRI) | datalog reason sqlite | 721.3 | 1000000 |
| tc smallworld-1000 (IRI) | n3 (eyeron) sqlite, incomplete | 417091.6 | 999221 |
| tc smallworld-1000 | datalog redb | 509.2 | 1000000 |
| tc smallworld-1000 | datalog redb evaluation only | 275.0 | |
| tc smallworld-1000 | datalog redb results only | 196.0 | |
| goal smallworld-1000 | datalog redb (demand) | 1.4 | 1000 |
| tc smallworld-1000 | souffle interpreter (incl. start) | 440.2 | 1000000 |
| contagion 2000 contacts | datalog sqlite | 5.9 | 2001 |
| contagion 2000 contacts | datalog redb | 3.0 | 2001 |

Soufflé's chain time varies between runs (449 ms on 2026-10-28, 610 ms here). Against
the faster 2026-10-28 Soufflé times the closures are 1.4× (chain), 1.4× (hub) and
1.2× (small world) on redb.

### What changed

A profile (`sample`, release build, all closure workloads) on 2026-10-29 before the
change put about 70 % of `View::datalog` in `goal_rows`, which decoded every cell twice
and sorted 10⁶ rows by comparing `Value`s, and about 29 % in evaluation. The change
(`lat.md/changes.md#Tune Datalog Performance`):

- decodes each distinct id once and sorts rows by integer ranks in the same value order;
- short-cuts interval sets that cover every instant;
- drops derivations already covered at emit;
- finds rows through a table of row numbers instead of a key copy per tuple;
- resolves each rule variant's loaded predicates and indexes once;
- caches plans per variant until a relation grows tenfold.

After it, evaluation is about two thirds of a closure query. The rest of the result
cost is mostly cloning and freeing 2 × 10⁶ IRI strings in the rows, which only a change
of the result types (shared strings) would remove.

## Baseline (2026-10-28)

Measured on 2026-10-28, Apple M2 Max, Soufflé 2.x interpreter:

| workload | engine | ms | facts |
|---|---|---:|---:|
| tc chain-1500 | datalog sqlite | 1693.6 | 1124250 |
| goal chain-1500 | datalog sqlite (demand) | 12.3 | 1498 |
| sg chain-1500 | datalog sqlite | 11.6 | 1499 |
| tc chain-1500 | datalog redb | 1585.6 | 1124250 |
| goal chain-1500 | datalog redb (demand) | 2.7 | 1498 |
| sg chain-1500 | datalog redb | 1.9 | 1499 |
| tc chain-1500 | souffle interpreter (incl. start) | 449.3 | 1124250 |
| tc hub-2000x200 | datalog sqlite | 681.5 | 422100 |
| goal hub-2000x200 | datalog sqlite (demand) | 8.3 | 201 |
| tc hub-2000x200 (IRI) | datalog reason sqlite | 2594.7 | 422100 |
| tc hub-2000x200 (IRI) | n3 (eyeron) sqlite | 17175.2 | 422100 |
| tc hub-2000x200 | datalog redb | 650.7 | 422100 |
| goal hub-2000x200 | datalog redb (demand) | 0.4 | 201 |
| tc hub-2000x200 | souffle interpreter (incl. start) | 157.4 | 422100 |
| tc smallworld-1000 | datalog sqlite | 1486.2 | 1000000 |
| goal smallworld-1000 | datalog sqlite (demand) | 11.0 | 1000 |
| tc smallworld-1000 (IRI) | datalog reason sqlite | 7617.3 | 1000000 |
| tc smallworld-1000 (IRI) | n3 (eyeron) sqlite, incomplete | 491073.9 | 999221 |
| tc smallworld-1000 | datalog redb | 2109.1 | 1000000 |
| goal smallworld-1000 | datalog redb (demand) | 1.6 | 1000 |
| tc smallworld-1000 | souffle interpreter (incl. start) | 481.9 | 1000000 |
| contagion 2000 contacts | datalog sqlite | 6.5 | 2001 |
| contagion 2000 contacts | datalog redb | 3.6 | 2001 |

## Gate verdicts

The proposed gates of decision DL20 (`lat.md/datalog.md#Gates And Oracles`):

| Gate | Result | Verdict |
|---|---|---|
| ≥ 10× `tm-n3` on the same Horn programs | hub closure 84× (234 ms against 19.7 s; 6.6× on 2026-10-28); small world: N3 gave up incomplete after 417 s, Datalog finished in 0.72 s | met |
| ≤ 2× Soufflé's interpreter on closure and same-generation | closure 1.0× (chain), 1.4× (hub), 1.2× (small world) on redb; at most 1.4× against the 2026-10-28 Soufflé times (3.1–4.3× before) | met |
| enforced constraints ≤ 20 % on a 100-statement transaction at 10⁶ | +15 % on redb (2.45 → 2.81 ms), within noise on SQLite (3.78 → 3.10 ms), when every row seeds the check; 0 checks when no row fits the constraint's atoms | met |

Where it already wins: goals with constants run for demand in 0.4–12 ms where the full
closure takes 0.6–2 s, same-generation over the chain takes 2–12 ms, and the DatalogMTL
contagion run over 2 000 timed contacts takes 4–7 ms in one pass, with no
segment-by-segment re-run.

Not yet measured: the RDFS/OWL 2 RL subset on WatDiv, a temporal LUBM-style set, 10⁶
statements, Nemo and MeTeoR (neither installed). Further engine speed needs the sorted
columnar relations with merge and leapfrog joins of DL12 (task 4.11 of
`add-datalog-engine`); the probe-or-load threshold and morsel size are still untuned
(task 9.3).

## Commit overhead of enforced constraints

`cargo run --release -p tm-bench --features datalog --example datalog_constraints -- --size 1000000 --reps 5`
(OpenSpec change `add-datalog-constraints`, task 4.4): a 100-statement commit of new
`v:role` statements on a store of 10⁶ `v:role` statements, without enforcement, with
`one_ceo` enforced (no row fits its `v:CEO` atoms, so nothing is checked), and with
`constraint banned error :- v:role(X, R), v:banned(R, true).` enforced (every row seeds a
probe). Best of five, milliseconds:

| backend | store | enforced | ms per 100-statement commit | checks | seed rows |
|---|---:|---|---:|---:|---:|
| sqlite | 1000000 | no | 3.78 | 0 | 0 |
| sqlite | 1000000 | yes (no row fits) | 1.78 | 0 | 0 |
| sqlite | 1000000 | yes (every row seeds) | 3.10 | 1 | 100 |
| redb | 1000000 | no | 2.45 | 0 | 0 |
| redb | 1000000 | yes (no row fits) | 2.55 | 0 | 0 |
| redb | 1000000 | yes (every row seeds) | 2.81 | 1 | 100 |

Commit-time checks plan without reading the store's statistics and never load a
predicate: before that, each check counted up to 4 096 rows per atom through SQLite and
the seeded commit took 76 ms.
