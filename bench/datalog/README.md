# Datalog benchmark

`cargo run --release -p tm-bench --features datalog,n3 --example datalog -- --noise 100000 --reps 2`
(`crates/tm-bench/examples/datalog.rs`, OpenSpec change `add-datalog-engine`, task 9).
Every store holds the graph's `v:e` edges plus 100 000 `v:name` literals that no rule
reads. Before a time is printed, SQLite and redb must return the same facts, Soufflé
(when `souffle` is on the PATH) must agree on the closure, and N3 must agree on the
IRI form of the closure when it completes. Times are the best of the repetitions, in
milliseconds; Soufflé's include its process start and fact loading.

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
| ≥ 10× `tm-n3` on the same Horn programs | hub closure 6.6× (2.6 s against 17.2 s); small world: N3 gave up incomplete after 491 s, Datalog finished in 7.6 s | not met on the hub |
| ≤ 2× Soufflé's interpreter on closure and same-generation | closure 3.5× (chain), 4.3× (hub), 3.1× (small world) | not met |
| enforced constraints ≤ 20 % on a 100-statement transaction at 10⁶ | measured by `add-datalog-constraints` | open |

Where it already wins: goals with constants run for demand in 0.4–12 ms where the full
closure takes 0.6–2 s, same-generation over the chain takes 2–12 ms, and the DatalogMTL
contagion run over 2 000 timed contacts takes 4–7 ms in one pass, with no
segment-by-segment re-run.

Not yet measured: the RDFS/OWL 2 RL subset on WatDiv, a temporal LUBM-style set, 10⁶
statements, Nemo and MeTeoR (neither installed). The closure is bound by tuple-at-a-time
insertion and interval-set handling in the semi-naive loop; tuning it (and the
probe-or-load threshold) is task 9.3, and the worst-case-optimal join for cyclic bodies
is task 4.11.
