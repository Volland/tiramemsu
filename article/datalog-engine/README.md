# Article: Building a Datalog engine for a bitemporal store

A long read (about 1,900 words, no images, no tables) on the engineering of `tm-datalog`: why it is not an embedded engine, interval sets carried through joins, how the temporal operators were checked against brute-force definitions, four things the tests found (records versus facts, a rule that negates its own head, planning by counting the store, a clock that panics on wasm), seeded constraint checks, and the benchmark with the two gates that were missed.

It appears on Substack, from this folder, and on the website as `site/articles/building-the-datalog-engine.html`. It complements `article/datalog/`, which explains what the engine does.

| File | What it is |
|---|---|
| `building-the-datalog-engine.md` | The article, the source of truth. No tables |
| `build-site.py` | Rebuilds the website page (needs `pandoc`): `python3 build-site.py` |

## Where the claims come from

- Decisions: `lat.md/datalog.md` (DL1 to DL26).
- Numbers: `bench/datalog/README.md` (run of 2026-10-28, Apple M2 Max). A rerun means updating the figures here.
- Tests named in the text: `crates/tm-datalog/src/interval.rs` (unit tests against brute force), `crates/tiramemsu/tests/datalog.rs` (`instant_by_instant_oracle`) and `datalog_constraints.rs` (`seeded_check_equals_full_check`).
