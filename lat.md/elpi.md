---
type: Decision
openspec: [elpi-engine, elpi-programs, elpi-constraints, elpi-materialization, elpi-query-calls]
---
# ELPI

Decisions E1–E13 of the ELPI interview of 2026-10-31: λProlog programs run by lean-elpi over FFI as a third reasoning engine, with an engine crate and a persistence crate.

[lean-elpi](https://github.com/Volland/lean-elpi) implements the ELPI dialect of λProlog in Lean 4 and ships `libleanelpi` with a C interface and the Rust crate `leanelpi`. E1–E10 were agreed one question at a time; E11–E13 are proposed defaults that were not interviewed. All four changes of [[elpi#Prerequisite And Four Changes]] are built ([[crates/tm-elpi/src/lib.rs]], [[crates/tm-elpi-store/src/lib.rs]], the facade's [[crates/tiramemsu/src/elpi.rs]] and [[crates/tiramemsu/src/elpi_materialize.rs]]) against a local lean-elpi 0.4 checkout; change 4 added SPARQL and Cypher calls, the bridge ops and the MCP tools. N3 ([[reasoning]]) and Datalog ([[datalog]]) stay; ELPI adds higher-order clauses, hypothetical reasoning (`=>`, `pi`) and constraint handling rules over the same views and the same derived beliefs.

## Four Uses Over The C Interface

E1: ELPI serves goal queries, check-only validation, commit-time enforcement and materialization in v1, through lean-elpi's C interface over FFI.

- motivated_by:: [[datalog#One Language Three Uses]] {weight: primary, since: "2026-10-31"}
- motivated_by:: [[constraints#No Unsafe Code]] {weight: secondary, since: "2026-10-31", note: "the FFI and its unsafe stay inside leanelpi"}

Rejected: a query-only first release, and linking lean-elpi's Lean code without the C interface.

## Host Callback Builtins

E2: programs read the store through host builtins added to lean-elpi's C ABI (0.4): `tm.triple S P O E`, `tm.added`, `tm.retracted` and `tm.valid E From To`; bound arguments become prefix scans through `ReadStore`.

- motivated_by:: [[constraints#Single Writer]] {weight: primary, since: "2026-10-31", note: "delta-seeded checks need lazy, bounded reads"}
- motivated_by:: [[reasoning#Optional Crate]] {weight: secondary, since: "2026-10-31"}

- changed_by:: [[changes#Add ELPI Engine]] {date: 2026-10-31}

As built in lean-elpi 0.4, `leanelpi_declare(program, name, signature)` declares a host predicate and `leanelpi_solve_host` calls one host function for every call of one, synchronously on the calling thread, with all rows of a call at once (not lazily). The prelude also declares the builtins, so a program's clauses can call them ([[crates/tm-elpi/src/run.rs#PRELUDE]], [[crates/tm-elpi/src/host.rs#Host]]). Rejected: loading the scope as `triple` facts through `leanelpi_load` and `leanelpi_assert` with 0.3.3 unchanged, which costs time in the scope's size on every run and cannot be seeded by a delta.

## Program States Its Support

E3: a materialized conclusion comes from `tm.conclude Triple Support`, its support being the listed eids plus the program's source statement; the host refuses eids no callback returned, eids not live, and an empty list.

- motivated_by:: [[reasoning#Rules Are Support]] {weight: primary, since: "2026-10-31"}
- motivated_by:: [[constraints#Never Forget]] {weight: primary, since: "2026-10-31"}

- changed_by:: [[changes#Add ELPI Materialization]] {date: 2026-10-31}

Backtracking, cut, `=>` and higher-order calls make "the facts a solution used" no by-product of a run, so the author threads the witnesses, as λProlog programs already do. Checking against the eids seen in the run means support cannot be forged. Rejected: tracing every eid the callbacks returned (failed branches over-approximate support) and proof terms from lean-elpi's LF layer in v1; proof terms may come later behind `proof: true`, as [[datalog#Provenance On Demand]] does.

## Reconciled Materialization

E4: `materialize` always reconciles as Datalog does; a scan of the clause bodies for `not`, `\+`, `!`, `std.findall` and `tm.retracted` labels a program `non_monotonic`, and the report lists such programs under `needs_rerun_after_writes`.

- motivated_by:: [[datalog#Shared Materialization With Reconciliation]] {weight: primary, since: "2026-10-31"}
- motivated_by:: [[reasoning#Monotonic Materialization]] {weight: secondary, since: "2026-10-31", note: "N11 refuses; refusing here would exclude idiomatic cut and findall"}

The scan is conservative: a wrong `non_monotonic` label costs a rerun hint, never a wrong store. Rejected: refusing such programs as N11 does, and reconciling without a label.

## Two Constraint Entry Points

E5: a program writes `tm.violation error Name Msg Eids` as its full check and `tm.violation-delta Name Msg Eids` as the check seeded by the transaction's delta; only the delta form runs at commit.

- motivated_by:: [[datalog#Warn And Error Constraints]] {weight: primary, since: "2026-10-31"}
- motivated_by:: [[constraints#Single Writer]] {weight: primary, since: "2026-10-31"}

- changed_by:: [[changes#Add ELPI Programs]] {date: 2026-10-31}

The full form runs for `check`, on registration and when enforcement is turned on; registering a program that live data violates is rejected with the violations. At commit the budget is lean-elpi's fuel plus a count of callback rows (as built: `max_seed_rows` rows, and the deadline turned into `timeout_ms × 4000` resolution steps, since lean-elpi runs a few million a second), and exceeding it fails the transaction with `ConstraintBudgetExceeded`. An `error` constraint without a delta form is check-only. A violation is kept only when the view as of the previous transaction did not already hold it. Rejected: a full check at every commit (cost grows with the store) and seeding the full form from each delta row by the host (works only for bodies that start with a recognisable `tm.triple`).

## Engine And Persistence Crates

E6: `tm-elpi` is the engine over `leanelpi` and `tm-core` and never writes; `tm-elpi-store` is persistence over `tm-elpi`, `tm-core` and `tm-store`; the facade feature `elpi`, off by default, wires them in.

- motivated_by:: [[reasoning#Optional Crate]] {weight: primary, since: "2026-10-31"}
- motivated_by:: [[constraints#No Unsafe Code]] {weight: primary, since: "2026-10-31"}

`tm-elpi` holds the callback builtins over a `&dyn ReadStore`, the term mapping of [[elpi#Structured Terms]], query, check and conclude runs and the support check of [[elpi#Program States Its Support]]; it keeps `forbid(unsafe_code)`. `tm-elpi-store` owns program nodes, includes, enforcement on registration and at commit, and builds a materialization plan (conclusions with support, the reconciliation diff) that the facade applies through its shared `materialize.rs`, so snapshot then commit stays one path for three languages. Unlike N3 and Datalog, whose persistence lives in facade modules, this keeps persistence in its own crate. As built, `tm-elpi`'s cargo feature `native` links lean-elpi; without it the pure parts build and every run is `Unsupported`, so `cargo test --workspace` needs no native library. Rejected: one `tm-elpi` crate with facade modules as Datalog has.

## Program Nodes With Includes

E7: a program is a node with one `elpi:source` statement (`elpi:` is `urn:tiramemsu:elpi:`), edited by supersede and enforced by `(program elpi:enforced true)`; `accumulate <program-iri>` resolves to that program's live source.

- motivated_by:: [[datalog#Program Nodes]] {weight: primary, since: "2026-10-31"}
- motivated_by:: [[reasoning#Rules Are Support]] {weight: primary, since: "2026-10-31"}

Includes resolve from the same snapshot; a cycle fails with `ProgramCycle`. Every included source statement joins the support of what the program materializes, so editing a library retracts what depended on it. `program_info` lists the closure. A node that also holds `n3:source` or `dl:source` fails with `RuleLanguageConflict`. Rejected: self-contained programs only, and expanding includes once when a program is stored.

## Structured Terms

E8: RDF terms cross as `(iri "…")`, `(lit Lex Datatype)`, `(lang Lex Tag)` and `(bnode Id)`, with `xsd:integer`, `xsd:double` and `xsd:string` as native ELPI values; eids are `(eid N)` and intervals `(interval From To)`.

- motivated_by:: [[datalog#Goal Calls In SPARQL And Cypher]] {weight: secondary, since: "2026-10-31"}

- changed_by:: [[changes#Add ELPI Engine]] {date: 2026-10-31}

ELPI is typed, so as built each native value sits under a one-argument constructor of type `term`: `(num N)`, `(dbl F)`, `(str S)`, with `(bool S)`, `(node N)`, `(tx N)` and `(stmtv (eid N))` beside them; matching `(num N)` binds an ELPI `int` ([[crates/tm-elpi/src/term.rs#from_value]]). The 0.4 ABI returns bindings as a JSON tree (constructor, arguments, native values), so decoding never parses printed ELPI. A value with a dictionary id comes back as that term; a computed value without one is a `Value` from `View::elpi` and fails with `Unsupported` in SPARQL and Cypher calls. Rejected: opaque `(t id)` handles (no inline constants) and structured terms over 0.3.3's printed strings.

## Pointwise Valid Time

E9: as N3 does, a conclusion holds at an instant when it follows from the statements valid then; `query` and `check` run at one `valid_at`, and `materialize` and the full check sweep the scope's segments.

- motivated_by:: [[reasoning#Pointwise Valid Time]] {weight: primary, since: "2026-10-31"}
- motivated_by:: [[derived-beliefs#Widening Is A Supersede]] {weight: secondary, since: "2026-10-31"}

- changed_by:: [[changes#Add ELPI Engine]] {date: 2026-10-31}

Transaction time is the view's pinned as-of. As built, `conclude` reads every statement of the view whatever its valid time and gives a conclusion the intersection of its stated support's intervals, dropping an empty one: with the support explicit, that is the pointwise result for programs the scan of [[elpi#Reconciled Materialization]] accepts. A `non_monotonic` program is swept per segment, cut at the bounds of every statement the first run read ([[crates/tm-elpi/src/run.rs#conclude]]). Constraint checks read every statement the same way, and a violation holds over the intersection of the cited statements. In a segment `tm.triple` returns the statements valid throughout it; the host gives a conclusion the merged union of the segments that derived it and widens it by supersede. `tm.valid` exposes raw intervals for explicit temporal reasoning, but programs never choose a conclusion's interval. A delta check sweeps only the segments the delta's intervals touch. Rejected: program-chosen intervals checked for containment (a later opt-in `tm.conclude/3` at most) and ignoring valid time.

## Native Library Distribution

E10: `leanelpi` 0.4 is published to crates.io and finds `libleanelpi` through `LEANELPI_DIST`, then `pkg-config`, or an opt-in checksum-pinned `download` feature; `elpi` is off by default on every crate, and the CLI and MCP binaries ship it first.

- motivated_by:: [[constraints#Publishable Crates]] {weight: primary, since: "2026-10-31"}
- motivated_by:: [[reasoning#Eyeron From Crates.io]] {weight: secondary, since: "2026-10-31"}

`libleanelpi` carries the Lean runtime and exists for darwin-arm64/x64 and linux-arm64/x64 only, so the CLI keeps `elpi` out of its defaults (`cargo install` must not need the library), and on wasm and Windows the bridge ops answer `Unsupported`. Release binaries of `tm` and the MCP server for the four platforms link it statically. Node and Python packages follow later, which departs from [[datalog#Own Calls On Every Surface]] because of the platform gap. One CI job on linux-x64 and macos-arm64 downloads the release and runs clippy and the tests with `--features elpi`. A missing library fails the build with a message rather than a panic; docs.rs builds through `DOCS_RS`. Rejected: Node and Python packages with ELPI from the start.

Status: lean-elpi 0.4 is built locally and not yet published, so the workspace has a `[patch.crates-io]` entry pointing at `../lean-elpi/bindings/rust`; until it is removed, CI cannot resolve the workspace without that checkout. lean-elpi's `build.rs` copies the library into its `OUT_DIR`, which cargo puts on the loader path of `cargo test` and `cargo run`; an installed `tm` needs the library on its loader path. Not built yet: the `pkg-config` lookup, the `download` feature, static linking of release binaries and the CI job.

## Own Calls

E11 (proposed, not interviewed): ELPI has its own calls behind the `elpi` feature: `View::elpi`, `View::elpi_check`, `View::elpi_conclude`, `View::elpi_program_info`, `TxElpi::store_elpi_program`, `TxElpi::enforce_elpi` and `Db::materialize_elpi`.

- motivated_by:: [[datalog#Own Calls On Every Surface]] {weight: primary, since: "2026-10-31"}

The JSON bridge gains `elpi`, `elpiCheck`, `elpiProgramInfo`, `storeElpiProgram`, `enforceElpi` and `materializeElpi`, reusing N3's `scope`, `window`, `limits`, `budget` and `dryRun` shapes plus `fuel`. The MCP server gains `elpi_query` and `elpi_check`, write tools hidden in read-only mode, and the CLI `tm elpi`. - changed_by:: [[changes#Add ELPI Query Calls]] {date: 2026-10-31}

Built: the Rust calls, the CLI `tm elpi query|check|conclude|materialize|program put|show|enforce|unenforce` ([[crates/tiramemsu-cli/src/elpi.rs#elpi]]), the bridge ops ([[bindings#JSON Bridge#ELPI]]), the five MCP tools, and goal calls ([[query#Datalog Calls]]). Not built: Node, Python and wasm wrappers. Goal calls from SPARQL `SERVICE <urn:tiramemsu:elpi:query>` and Cypher `CALL tiramemsu.elpi.query` share Datalog's call path: the IR leaf carries a `GoalLanguage`, and the engine holds one provider per language (refined while building: no second provider trait). Rejected: a language parameter on the N3 or Datalog calls.

## Oracles And Gates

E12 (proposed, not interviewed): three oracles check results: `tm-datalog` and `tm-n3` on Horn programs, lean-elpi's conformance suite against ELPI 1.18, and delta-against-full property tests of constraints.

- motivated_by:: [[datalog#Gates And Oracles]] {weight: primary, since: "2026-10-31"}

The Horn programs are written in each language and must give the same conclusions. The property test runs each `tm.violation-delta` and its full `tm.violation` on random transactions and compares the new violations.

Built: the Datalog oracle as a test on a pseudo-random graph ([[crates/tiramemsu/tests/elpi.rs#datalog_oracle_agrees]]) and the delta-against-full property test ([[crates/tiramemsu/tests/elpi_props.rs]]), which checks that an enforced program refuses a random transaction exactly when the full check finds a violation it introduced (300 cases pass; the suite runs 24). Not built: the N3 oracle and the benchmark suite. A `tm-bench` ELPI suite (closure, a delta-seeded constraint, a hypothetical-reasoning validator) on both backends reports costs beside Datalog and N3; no speed gate is set before the first numbers exist, since lean-elpi is about 2.4× slower than ELPI 1.18.

## Prerequisite And Four Changes

E13 (proposed, not interviewed): a lean-elpi 0.4 release, then four OpenSpec changes in this repository, with enforcement before materialization: engine, programs and constraints, materialization, query calls.

- motivated_by:: [[datalog#Four Changes]] {weight: primary, since: "2026-10-31"}

0. lean-elpi 0.4, in its own repository: host builtins of [[elpi#Host Callback Builtins]], JSON-tree results of [[elpi#Structured Terms]], `build.rs` discovery and the `download` feature of [[elpi#Native Library Distribution]], and publication on crates.io.
1. `add-elpi-engine`: `tm-elpi` (builtins, terms, runs, the monotonicity scan, segment sweep), `View::elpi` and `View::elpi_check`, the bridge read ops, MCP read tools, `tm elpi`, the CI job and the oracles.
2. `add-elpi-programs`: `tm-elpi-store` with program nodes, includes, `TxElpi::store_program`, `TxElpi::enforce` and commit-time enforcement.
3. `add-elpi-materialization`: the plan in `tm-elpi-store`, `Db::materialize_elpi` through the shared module, reconciliation.
4. `add-elpi-query-calls`: SPARQL `SERVICE` and Cypher `CALL`.

Rejected: one change, and materialization before constraints.
