# Project review — tiramemsu 0.4.0

Reviewed on 2026-10-10 at commit `5a55efc` (release 0.4.0 plus paper and site work). This is the second full review; the first (2026-10-03, against 0.1/0.2) proposed ten changes, and all ten shipped in 0.3.0 and are archived under `openspec/changes/archive/2026-10-04-*`. This review starts from that state.

The review covered four areas in parallel — storage and transactions (`tm-core`, `tm-rusqlite`, the `tiramemsu` facade), the query stack (`tm-ir`, `tm-exec`, `tm-sparql`, `tm-cypher`), the outward surface (MCP server, JSON bridge, Python/Node/WASM bindings, CI, README), and repository health — and every finding below was checked against the source. Reviewer claims that did not hold up are listed at the end.

## Scorecard

| Area | Score | One-line verdict |
|---|---|---|
| Architecture and design | **9 / 10** | One IR, one temporal predicate writer, one host boundary; layers by statement id give provenance, correction and graphs a single model |
| Storage and transaction engine | **8 / 10** | Panic-safe transactions, id burning, invariant triggers in the file; remaining issues are performance, not correctness |
| Query stack | **7.5 / 10** | Injection-proof codegen, bounded path engine with completeness reporting; conformance gaps users will hit |
| Agent surface (MCP, bindings) | **8 / 10** | MCP server is unusually safety-conscious; bindings are idiomatic; a few asymmetries |
| Verification | **9 / 10** | Closed W3C and TCK harnesses, differential SPARQL/Cypher suite, proptests, never-forget source audit, feature matrix, a paper with proofs |
| Performance evidence | **6 / 10** | Good micro-benchmarks, but the scale ceiling is still unknown and the as-of churn target is unmet |
| Release and operations | **6 / 10** | Publishing pipelines are ready, but no CHANGELOG, npm not published, no hosted API docs |
| Repository hygiene | **6.5 / 10** | Clean crates; an orphaned Lean model at the repo root that cannot build as laid out |
| **Overall** | **8 / 10** | A carefully engineered, unusually well-specified embedded store. The next cycle should be about evidence at scale, recall quality and release polish, not more breadth |

## What works

**The core idea is right and is implemented once.** A statement is a row `(eid, s, p, o)` with its own id, and an id can be the subject or object of another statement. Provenance, confidence, beliefs, named graphs, statement graphs and metagraphs all fall out of that without a second data structure. Time is resolved in exactly one function (`tm_core::scan_predicates`), reached by views, dependents, graph membership, events and both query languages. This is the decision that keeps the system coherent as features are added.

**Transaction safety is production-grade.** Every write path (`transact`, `speculate`, derived writes, reads) catches unwinds, rolls back both SQLite and the term-dictionary overlay, and re-throws the original panic. Ids seen during a speculative transaction are burned so they are never reissued, with a fallback when the burn commit itself fails. The never-forget rule is enforced by SQLite triggers inside the file, so it holds even against external tools, and CI greps the engine source for forbidden `DELETE`/`UPDATE`.

**The generated SQL never contains data.** Every user value reaches SQLite as a bound `?N` parameter; `format!` in the codegen only interpolates generated aliases and fixed column names. The query reviewer found no exception.

**Malformed queries do not panic.** The `unwrap`/`expect` sites in the parsers, function evaluation and codegen are all guarded by preceding arity or presence checks.

**The path engine is bounded and honest.** Thompson NFA → DFA (state cap 4096), a configurable state limit (default 1 M), budget polling, and every result says whether it was `Exhaustive`, `StoppedAtBound` or `StoppedAtCap`. Time-respecting search is cross-checked against a brute-force oracle.

**The conformance harnesses are closed in both directions.** The W3C SPARQL runner and the openCypher TCK runner fail on any unlisted deviation *and* on any listed deviation that starts passing, so the deviation lists can only change by an explicit edit, each with a reason.

**The MCP server is designed for untrusted agents.** Tool schemas use `additionalProperties: false`, path keys are refused, raw SQL is refused, SPARQL/Cypher writes are rejected before a transaction opens, read-only mode removes write tools from `tools/list` *and* refuses them, and budgets default to 30 s / 10 000 rows / 8 MiB. Errors are structured (`ReadOnly`, `PathNotAllowed`, `DeadlineExceeded`) so an agent can act on them, and the `initialize` instructions orient an LLM well.

**Design, specs and code are kept in sync.** `lat.md/` is a checked knowledge graph with code references, `openspec/specs/` has 47 capability specs, and the paper formalises the cascade, grounding and supersede guarantees, with four conformance gaps it found repaired in 0.4.0 (storage format 4).

**Pay-for-what-you-use builds are real.** `default-features = false` links no parser, and the CI feature matrix checks six combinations, including dependency trees.

## What to improve

Findings are ranked by user impact. None is a data-loss or correctness bug in normal operation; the High items are about evidence and adoption rather than defects.

### High

**H1. The scale ceiling is still unknown, and the as-of churn target is not met.** `lat.md/overview.md#Open Inputs` still lists the scale ceiling as unknown, so the benchmark targets in `lat.md/roadmap.md#Benchmarks` are unset. The last churn measurement (2026-10-03) showed as-of lookups going from 2.75 µs to 81.5 µs between 1 and 1 000 versions per key (about 30×), against a target of ≥ 70 % of the no-history baseline, and the current view also regressed at 1 000 versions with the cause unisolated. Nothing since has re-measured it, and `crates/tiramemsu/benches/README.md` says its path table is an older run. For a "never forget" store, history depth *is* the workload. Fix: pick a target (for example 10⁷ statements, 1 000 versions per hot key), run the churn, point, 2-hop and path benchmarks there, isolate the current-view regression (page cache vs. index choice vs. layout), and record the numbers as release gates.

**H2. No CHANGELOG, and the npm package is not published.** A user going from 0.3 to 0.4 cannot see that format 4 adds date guards or that `assert` now rejects non-live endpoints, both of which change behavior. `npm install @tiramemsu/node` fails, even though `npm-publish.yml` is ready. Fix: add `CHANGELOG.md` with entries back to 0.1, including storage format changes and behavior changes, and run the npm release.

**H3. Cypher has no temporal types in a bitemporal database.** 1 064 of the 1 265 TCK deviations are one line: "temporal: durations, time/localtime, map constructors, truncation…". Users can ask `MATCH TIME RESPECTING`, but cannot write `datetime('2026-10-01')`, `duration('P1D')` or compare dates idiomatically. For a product whose pitch is time, this is the most visible gap in the second query language. Fix: implement `date`, `datetime`, `localdatetime`, `duration` and their comparison and arithmetic, mapped onto the existing instant and valid-time values; the TCK list shows exactly which scenarios become green.

### Medium

**M1. Supersede loads its cascade set row by row.** `crates/tm-core/src/engine/supersede.rs:124-127` issues one `SELECT … WHERE eid = ?` per cascade member. The 2026-10-03 measurement put a dry-run supersede at 53.7 ms for a 1 000-statement cascade. Fix: load the set in one query (`WHERE eid IN rarray(?)` where the host has the vtab, chunked `IN (…)` otherwise).

**M2. The term-dictionary cache clears itself on large transactions.** `crates/tm-core/src/term.rs:141-147`: when `committed + overlay > 2 × capacity`, the whole committed cache is dropped. One bulk-import chunk with more new terms than the capacity (16 384 by default) discards the warm cache, and later transactions pay a SQL lookup per term until it refills. Fix: bounded LRU eviction, or cap only the committed side.

**M3. Bounded writer acquisition spin-waits.** `crates/tiramemsu/src/db.rs:369-383` polls `try_lock` with a 1 ms sleep when a budget or timeout is set. It adds up to 1 ms latency per attempt and burns CPU under contention. Fix: a `Condvar` notified on writer release, or `parking_lot`'s `try_lock_for`, with the interrupt check on wake.

**M4. LFTJ materialises its inputs with no memory bound.** `crates/tm-exec/src/lftj/mod.rs:194-206` loads every pattern's matching rows into a `Vec<i64>`; it polls the deadline per row but charges no bytes. Routing requires patterns to be *large* (`min_rows`, default 100 000) but sets no upper bound. Fix: charge the byte budget while loading, and add a `max_input_rows` routing limit that falls back to SQL.

**M5. SPARQL conformance gaps on common functions.** 147 of 781 W3C tests are listed deviations (about 81 % pass). The ones users will hit: decimal arithmetic returns `xsd:double` (11 tests), string functions drop language tags (10), negated property sets `!p` are `Unsupported`, and `xsd:date` drops its time zone (4). Relative IRIs are rejected by design. Fix the decimal and language-tag groups first; they are silent wrong-type results, not loud errors.

**M6. The Lean store model is orphaned at the repository root.** `Store/*.lean` and `Sqlite/Conn.lean` (918 lines, tracked) import `Tiramemsu.Store.*`, but there is no `lakefile`, no `lean-toolchain`, and the files are not under a `Tiramemsu/` directory, so they cannot build as laid out. They carry `@lat: [[architecture#Store Abstraction#Model Store]]`, but that section does not exist, and `lat check` passes anyway, so nothing catches it. The paper-2 Lean project under `paper/metagraphs-from-identified-triples/lean/` shows the intended setup. Fix: move them to `lean/Tiramemsu/{Store,Sqlite}/` with a lakefile and toolchain, add the missing lat section or fix the reference, and build it in CI.

**M7. Python `TxBuilder.new_node()` drops its result.** `bindings/python/python/tiramemsu/_db.py:264-266` returns `None`, so the allocated node cannot be referenced later in the same transaction. Node's `Tx.newNode()` returns a `Ref`. Fix: return `Ref(self._fresh())` with an `"as"` label, as `create` and `assert_` do.

**M8. Empty nodes and dual-view surprises for Neo4j users.** 60 TCK scenarios fail because `CREATE ()` with no properties leaves no trace (a node exists only through statements), and 46 because a relationship variable can stand in node position. Both are by design, but users porting Cypher will hit them in the first hour. Fix: a short "Cypher for Neo4j users" page, and optionally make `CREATE ()` assert an `rdf:type`-like existence statement under a setting.

### Low

- **L1.** `ObjectId::from_raw(r[k].as_i64().unwrap_or(0))` turns an unexpected NULL into a ghost id `0` instead of an error (`engine/supersede.rs:30`, `storage/schema.rs:136,241,271`, `graph.rs:75`). Return `InvalidTerm` instead.
- **L2.** `expect("cascade member exists")` in `supersede.rs:125` panics on a corrupt file. The panic is caught and rolled back, but the caller gets a panic payload rather than `NotLive`.
- **L3.** An instant-based `as_of` embeds `(SELECT max(t) FROM tx WHERE instant <= ?)` once per pattern (`tm-core/src/view.rs:115-118`). Resolve the instant to a `t` once per read.
- **L4.** The post-`ANALYZE` `PRAGMA schema_version` bump (`storage/stats.rs`) forces every pooled reader to reload its schema, and would collide if two `Db` handles opened one file. Document single-handle ownership, or detect a second handle with a lock row.
- **L5.** Node's `_refSeq` ref counter is module-global (`bindings/node/lib/index.ts:859`); make it per `Tx` like Python's.
- **L6.** The README quick start references `conf` and `fact` without defining them; `examples/quickstart.rs` is complete. Either show the full example or mark the snippet as an excerpt.
- **L7.** The Cypher prepass relies on non-local guards before `peek().unwrap()` (`tm-cypher/src/parse/prepass.rs`). Use `let … else` so a refactor cannot introduce a panic.
- **L8.** `tm-sparql` corrects `spargebra` 0.4.7's right-associative subtraction after parsing. Add a test that fails when upstream fixes it, so the correction cannot apply twice.
- **L9.** `lat.md/roadmap.md` called `reserve-replica-id` "a proposal only … waits for a decision", but its archived proposal says "decided (Option A) and implemented", and `lat.md/storage.md` documents the 48-bit counter bound. Fixed in `lat.md` with this review.
- **L10.** There is no hosted API reference (rustdoc on docs.rs covers Rust; nothing for Python or TypeScript).

## Validation run

`PROPTEST_CASES=32 cargo test --workspace --offline` and `cargo clippy --offline --workspace --all-targets -- -D warnings` were run for this review; results are recorded below.

- Tests: **1 311 passed, 0 failed, 1 ignored** across 139 test and doc-test suites (up from 1 127 at the first review). This includes the closed W3C SPARQL and openCypher TCK harnesses.
- Clippy with `-D warnings`: clean.
- `lat check`: passes after this review's `lat.md` updates.
- Not run: Node and Python packaging tests, WASM browser tests, benchmarks, and any dataset above 10⁵ statements. The performance statements above rely on the 2026-10-03 measurements and the recorded benchmark READMEs.

## Fix status (follow-up, 2026-10-10)

The 0.4.x items were fixed on the same day; entries are in `CHANGELOG.md` under Unreleased, regression tests in `crates/tiramemsu/tests/review_fixes.rs` (`lat.md/tests.md#Review Fixes`), and before/after numbers in `crates/tiramemsu/benches/README.md`. Workspace: 1 317 tests pass, clippy clean; Python 60/60 and Node 29/29.

| Finding | Status | Evidence |
|---|---|---|
| **New: STAT4 re-prepares every fixed-plan lookup** | Fixed | Found while isolating H1. With STAT4 samples SQLite re-prepares a statement on every bind of a bare `?N` against an indexed column. `tm-core` single-table lookups now write `col = +?N`. Current-view lookup after churn 16.6 → 2.7 µs; SPARQL point 55 → 20 µs, two-hop 92 → 45 µs (term lookups were paying it too) |
| H1 history depth | Partly fixed | The current-view regression is gone (above). As-of at 1 000 versions is still 66 µs because it scans every version of the key; the scale ceiling and gates remain in `add-scale-benchmarks` |
| H2 CHANGELOG / npm | CHANGELOG done | Retrofitted from 0.1.0; the npm release is still to be run |
| H3 Cypher temporal types | Specified | `openspec/changes/add-cypher-temporal-types` |
| M1 supersede row by row | Fixed | Cascade walk batched per frontier, rows loaded per 500 ids; order unchanged. 1 000-statement supersede 83 → 68 ms |
| M2 term cache clearing | Fixed | Segmented LRU; unit-tested. No measurable change on the hotspot fixture, whose cost is existence checks |
| M3 spinning writer wait | Fixed | Release-signalled condvar, 10 ms cancellation poll; tested. No throughput change (none was expected) |
| M4 LFTJ memory | Fixed | Inputs charged to `max_bytes`; tested on both routes |
| M5 SPARQL value semantics | Specified | `openspec/changes/fix-sparql-value-semantics` |
| M6 Lean model | Fixed | Moved to `lean/` with a lakefile; `lake build` passes; CI job added; lat sections restored |
| M7 Python `new_node` | Fixed, wider than reported | The bridge never bound `newNode`'s `as`, so Node's `Ref` did not resolve either. Fixed in the bridge and both bindings, with tests |
| M8 Neo4j surprises | Open | Documentation item |
| L1 NULL id columns | Not applicable | The columns are `NOT NULL` in a STRICT table; supersede's loader returns a typed error anyway |
| L2 cascade `expect` | Fixed | Missing rows are `NotLive` |
| L3 instant as-of | Not needed | Measured equal to transaction as-of (855 vs 854 µs) |
| L5 Node ref counter | Fixed | Per `Tx` |
| L6 README snippet | Fixed | |
| L9 replica-id drift | Fixed | `lat.md/roadmap.md` |
| L4, L7, L8, L10 | Open | |

## Reviewer claims that did not hold up

- "Platform `.node` binaries are checked into git." They are local build outputs and are not tracked (`git ls-files bindings/node` lists none).
- "The MCP server should answer `resources/list` and `prompts/list` with empty lists." The server advertises only the `tools` capability, so method-not-found is spec-compliant. Revisit only if resources are added (see the roadmap).
- "`Relation::at` in LFTJ needs checked indexing." It is an internal invariant, not reachable from user input; a `debug_assert!` is the most it warrants.

## Recommendation

Keep the architecture; it is the project's main asset. Do not add another query language, storage engine or network server yet. Spend the next cycle on three things, in order: **measure at scale and fix history-depth performance** (H1, M1, M2), **make the release legible** (H2, M6, M7), and **close the time gap in Cypher** (H3). The feature work that follows is in [feature-roadmap.md](feature-roadmap.md).
