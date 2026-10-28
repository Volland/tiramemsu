# Article: Datalog over time

A long read (about 4,400 words, no images, no tables) on Tiramemsu's Datalog engine `tm-datalog`: DatalogMTL operators and the 13 Allen relations over valid-time interval sets, stratified negation and aggregates, constraints enforced at commit (`ConstraintViolated`), stored programs materialized as derived beliefs with support, reconciliation, and a comparison with N3 reasoning in the same store. It says plainly that incremental materialization is not built and that two of the benchmark gates are not met.

It appears on Substack, from this folder, and on the website as `site/articles/datalog-temporal-reasoning.html`.

| File | What it is |
|---|---|
| `datalog-temporal-reasoning.md` | The article, the source of truth. No tables, because Substack cannot show them |
| `build-site.py` | Rebuilds the website page (needs `pandoc`): `python3 build-site.py` |

## Where the claims come from

- Decisions and design: `lat.md/datalog.md` (DL1 to DL26), `lat.md/reasoning.md` (N1 to N18), `CHANGELOG.md` [Unreleased].
- Requirements: `openspec/specs/datalog-*/spec.md` and `openspec/changes/add-datalog-materialization/`.
- Examples: each program is taken from a test in `crates/tiramemsu/tests/datalog.rs`, `datalog_constraints.rs` or `datalog_materialization.rs`; the test name is quoted next to it. Those tests run on SQLite and redb.
- Numbers: `bench/datalog/README.md` (run of 2026-10-28, Apple M2 Max). A rerun of `cargo run --release -p tm-bench --features datalog,n3 --example datalog` or of the `datalog_constraints` example means updating the figures in the article.

## Publishing on Substack

1. **Title:** Datalog over time: rules, constraints and derived beliefs in an agent's memory
2. **Subtitle:** the italic line under the title in the Markdown.
3. **Body:** convert the Markdown (for example with `build-preview.sh` of `article/cteg/`, adapted) and paste; check that code blocks came through as code.

### SEO description

Tiramemsu's Datalog engine: DatalogMTL and Allen relations over valid time, stratified negation, constraints enforced at commit, and materialized derived beliefs that fall with their premises, compared with N3.
