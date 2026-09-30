> Depends on change `add-core-store` (M0): `tm-core` (codec, term dictionary, `TimeRef` resolution, DDL, `volatile`), the reader pool / writer, `Db::with`, the facade `View`, the `Executor` trait with declared host capabilities and the `tm-rusqlite` host, and automatic planner statistics. Do not start until M0 is merged. Spec references: `specs/query-ir`, `specs/view-scoped-scans`, `specs/virtual-predicates`, `specs/sql-execution`. Design references: `design.md` D1–D16.

## 1. Workspace and crate scaffolding

- [x] 1.1 Add `crates/tm-ir` (dep: `tm-core`) and `crates/tm-exec` (deps: `tm-ir`, `tm-core` for the `Executor` trait and host registration hooks, `lru`, `regex`; no `rusqlite`, whose `functions`/`vtab` features stay in `tm-rusqlite`; dev: `insta`, `proptest`, and `tm-rusqlite` for integration tests) to the workspace, with empty module files per design D1; `cargo build` and `cargo clippy -D warnings` pass
- [x] 1.2 Add a shared test fixture crate or module (`tm-exec/tests/common`) that opens a temp `Db`, loads the alice/acme/globex/belief fixture from `lat.md/data-model#Layers`, and exposes the eids by name

## 2. tm-ir: types

- [x] 2.1 Implement `Var`, `TermOrVar`, `View`/`TxSel`/`ValidSel` with `View::overlay`, and `Semantics` with the `sparql()`/`cypher()` presets (D2); unit tests for overlay and presets (spec query-ir: Per-pattern view, Semantic flags)
- [x] 2.2 Implement `Op` and the eleven operator structs, including `TriplePattern.iso_group` / `include_volatile` and `PathPattern.max_hops` (D2)
- [x] 2.3 Implement `Expr`, `CmpOp`, `ArithOp`, `Func`, `Agg`/`AggFunc`, `Key`, `PathExpr`, `PathMode`, and `Params`
- [x] 2.4 Implement `vocab.rs` constants for `sys:subject|predicate|object` and `tm:txAdded|txRetracted|validFrom|validTo|retractKind`, and an `is_virtual(iri)` helper
- [x] 2.5 Implement the canonical S-expression printer (`display.rs`) for Op and PathExpr, with an `insta` snapshot test over a small corpus
- [x] 2.6 Implement the property-path text printer for PathExpr (the `tm_path` path grammar owned by add-path-engine: SPARQL 1.1 path syntax + `{m,n}`, IRIs/CURIEs, `sys:anyRelationship`), with round-trip tests against sample expressions
- [x] 2.7 Implement `IrBuilder` (fluent pattern/join/filter construction with a default View), used by all later tests

## 3. tm-ir: validation

- [x] 3.1 Implement `validate.rs`: Extend rebinding, Aggregate output/group collision, ragged Values, a virtual pattern with an eid, negative skip/limit → `InvalidQuery`; tests for each (spec query-ir: Structural validation)
- [x] 3.2 Implement the variable scoping pass (bound / maybe-missing sets per operator) and the "result column order" rule; tests for Project order and implicit first-binding order

## 4. tm-exec: planning front half

- [x] 4.1 Implement `plan/bind.rs`: substitute `Param` from `Params`, and `InvalidQuery` naming a missing parameter before any SQL runs; tests (spec query-ir: Parameters)
- [x] 4.2 Implement `plan/resolve.rs`: `TimeRef::Instant` → `t` through tm-core on the executing connection, and pre-history → Empty; tests with instants 1000/2000/3000 (spec query-ir: Time references resolved at plan time)
- [x] 4.3 Implement `plan/encode.rs`: inline codec, read-only dictionary lookup, and `Enc::{Id, Missing, Impossible}` for every position (D4); a test asserts the term table and `meta` are unchanged after planning
- [x] 4.4 Implement `plan/normalize.rs`: join flattening, Empty propagation per the D4 table, and the constant one-row result for a group-less Aggregate; one test per spec scenario under "Short-circuit on constants that cannot match"
  - Note: the plan tree (`plan::Node`) is built in `plan/normalize.rs` together with encoding and view resolution; joins are flattened there.
- [x] 4.5 Implement filter push-down with id-exact constant substitution only (no numeric substitution) and eid-constant → rowid equality (D4); tests including `?x = 1.0` against INT 1 staying a filter, and a datetime constant pushed down as the instant range `(ms << 15) | 7` … `(ms << 15) | 0x7FF7` with the tag check, while `sameTerm` with a datetime is substituted as id equality
  - Note: push-down is done at SQL level: a top-level conjunct lands in the WHERE of the relation binding its variable, and an id-exact constant compiles to `tN.x = ?` (datetime: `BETWEEN (ms<<15)|7 AND (ms<<15)|0x7FF7 AND (x & 15) = 7`), which gives SQLite the same seek as substituting into the pattern; the IR is not rewritten, so the variable keeps its column.
- [x] 4.6 Implement `plan/analyze.rs`: domains (Term / Computed with `ValueClass`), maybe-missing sets, and GYO cyclicity detection; unit tests on triangle vs chain vs star BGPs
  - Note: domains (`plan::analyze::{Dom, VClass}`) and maybe-missing flags are attached to each compiled column (`sqlgen::Col`) during codegen instead of a separate analysis pass; the IR-level bound / maybe-missing sets are `tm_ir::validate::scope`. GYO is `plan::analyze::is_cyclic`.

## 5. tm-exec: scans, views and virtual predicates

- [x] 5.1 Implement `scan.rs::view_predicates` exactly as in design D5 (the only writer of time predicates); unit tests assert the exact text for all six Now/AsOf/History × Unfiltered/At combinations, and that one parameter number is reused for `t`
  - Note: `scan::view_predicates` delegates to M0's `tm_core::scan_predicates` (after resolving instants to `t` at plan time), so the workspace keeps a single view→SQL mapping; it returns the combined conjunct as one `Vec` element.
- [x] 5.2 Add a CI grep test that fails if `t_ret`, `t_add`, `v_from` or `v_to` appear in string literals outside `scan.rs` and `virtual_pred.rs` (enforces the single mapping)
- [x] 5.3 Implement `virtual_pred.rs`: the column/tag mapping table, `IS NOT NULL` for nullable columns, constant-object column compares, and wrong-kind/non-STMT → Impossible (D6)
- [x] 5.4 Implement virtual-predicate alias reuse for an eid bound in an identical view, and otherwise an eid rowid lookup with its own view predicates; golden test showing exactly one `triple` alias for `?r tm:txAdded ?t` (spec virtual-predicates: resolve to row columns)
- [x] 5.5 Implement the volatile derived table for opted-in patterns under `{Now, Unfiltered}` only (D6); tests for all six scenarios of spec virtual-predicates "Volatile keys as virtual properties under Now only"

## 6. tm-exec: SQL functions and native operator interface

- [x] 6.1 Implement `udf.rs`: `tm_kind`, `tm_num`, `tm_str`, `tm_sortkey`, `tm_vkey`, `tm_min_by`/`tm_max_by` and `regexp`, defined host-neutrally and registered through the host hooks as deterministic/innocuous; datetime keys use the instant `id >> 15`; unit tests that sort keys order INT/DOUBLE/DECIMAL mixed, negative datetimes, equal instants with different offsets as equal keys, strings in code-point order, and lang strings after equal plain strings
- [x] 6.2 Implement `native.rs`: the `NativeOperator` trait, `OperatorRegistry`, `PlannerOptions`/`LftjConfig { enabled: false }`, and `register_all(conn)`
- [x] 6.3 Implement `plan/route.rs` per the design D7 pseudocode: path → Native (or `Unsupported` if unregistered), end-bound orientation via `Inverse`, no bound endpoint → `Unsupported`, cyclic BGP → Sql with `CyclicLftjDisabled`; tests for each route
- [x] 6.4 Implement `host.rs` (D1, decision D22): check the declared capabilities `functions` and `vtab` before registering anything, and fail `Db::open` with a typed error naming the missing capability; register the UDFs and native operators through the host hooks; tests with test hosts lacking `vtab` and lacking `functions`, and a `tm-rusqlite` test that a value comparison works on a pooled reader and inside `with` (spec sql-execution: Host capabilities)

## 7. tm-exec: SQL generation

- [x] 7.1 Implement the `Rel`/`Block` model with deterministic alias (`t/d/p/q/x`) and parameter (`?N`) allocation and `c0…cn` output aliases (D8)
- [x] 7.2 TriplePattern codegen: constants, repeated variables, shared-variable equalities, view predicates placed in WHERE or ON
- [x] 7.3 Canonical-eid predicate for `SetOfTriples` without an eid variable, plus elision under a root `Project{distinct}` (D10); tests for parallel edges, episodes and history re-assertion (spec sql-execution: Set versus bag semantics)
- [x] 7.4 Relationship isomorphism pairwise `<>`, with skipping of provably distinct predicate pairs (D11); tests for the self-loop, homomorphism and parallel self-loop scenarios
- [x] 7.5 Join/LeftJoin codegen, including right-side derived tables, ON-clause conditions and the Unbound vs Null3VL compatibility rule for maybe-missing variables (D11); tests for both "Join after optional" scenarios
- [x] 7.6 Filter/Expr codegen with SQL three-valued logic, value equality, class-checked ordering comparisons, missing constants bound as key BLOBs, datetime equality and ordering by the instant `id >> 15` (id equality kept for joins, `sameTerm` and `DISTINCT`), `Bound`, `In`, `Coalesce`, `If` and `Func`s (D9); tests for every scenario under "Filter truth values" and "Value equality and comparison", including "Same instant, different offsets" (`lat.md/tests#ObjectId#DateTime Keeps Its Offset`; its single `@lat` reference stays on the codec test in M0)
  - Note: constants missing from the dictionary that flow into columns (Values cells, LangStr/Typed constants) get plan-local ids; value operations read them through `Gen::term_source` (`term UNION ALL VALUES …`, bound parameters), so ordering/comparison works on them too. Missing strings, IRIs and numbers in expressions are compared by value (key BLOB / native number).
- [x] 7.7 `Expr::Exists(Op, negated)` codegen as a correlated `[NOT] EXISTS (SELECT 1 …)` with equalities on shared variables, and Empty nested trees folded to constant false/true; tests for every scenario under spec query-ir "Existence tests" (the semi/anti-join that `add-sparql-frontend` uses for FILTER [NOT] EXISTS and MINUS)
- [x] 7.8 Union (UNION ALL with NULL padding), Extend (Term vs Computed domain), Project (DISTINCT) and Values (VALUES derived table, UNDEF → NULL, unit relation) codegen
- [x] 7.9 Aggregate codegen per the D12 table, including HAVING placement and `collect` with a FILTER; tests for every "Aggregates" scenario
- [x] 7.10 OrderLimit codegen: sort keys, NULLS FIRST/LAST by `missing`, tie-breakers, hoisting of root order, `LIMIT -1 OFFSET ?`, and bound skip/limit; tests for every "Ordering by decoded value" scenario plus skip-only and limit 0
- [x] 7.11 Path region codegen: a `tm_path(?, ?, ?, ?, ?) AS pN` FROM item with correlated start, `"end"` equality, view text from `scan.rs`, and path text from `display.rs` (D7)

## 8. tm-exec: execution and decoding

- [x] 8.1 Implement `exec.rs`: `ExecContext { executor, cache mode, registry }` over `tm-core`'s `Executor` (the host's statement cache, e.g. `prepare_cached` in `tm-rusqlite`), parameter binding, row stepping, and ExecError mapping; the reader path wraps the whole pipeline in one `BEGIN … COMMIT`
- [x] 8.2 Implement `decode.rs`: inline decode via the codec, dictionary decode on the same connection, recursive `dt`, Computed and List cells, and `ExecStats` counters (D13)
- [x] 8.3 Implement `TermCache` (shared LRU, configurable capacity) and `ScopedCache` for speculation; tests for warm-cache zero dictionary reads, capacity bound, and cold/warm equality (spec sql-execution: Bounded term cache)
- [x] 8.4 Implement `explain()`: regions, short-circuit flag, SQL, params and `EXPLAIN QUERY PLAN` rows with the real parameters bound, both for the whole statement and per SQL region (`RegionInfo.query_plan`, D15), without stepping the query; tests for the normal, short-circuit and "Plan per region" scenarios

## 9. Facade integration

- [x] 9.1 Add `ConnSource::{Pool, Speculative}` to `View`, with `Db::with` handing out a Speculative view (D14); register UDFs and native operators through the host hooks (6.4) on the writer and on every pooled reader at open
  - Note: `Source::{Db, Writer}` in tiramemsu/src/view.rs is the ConnSource (Pool / Speculative); UDF and native-operator registration is installed at open through `Executor::registry()` (new M0 trait method with a default `None`; `HostRegistry` trait added to tm-core; rusqlite host implements it).
- [x] 9.2 Add `View::descriptor`, `View::execute_ir` and `View::explain_ir`, re-export the IR/result types, add `OpenOptions::{term_cache_capacity, planner}` and the hidden `with_native_operator`, and add the `Error::InvalidQuery { msg }` variant
  - Note: `Error::InvalidQuery` and `Error::MissingCapability` were added to tm-core's Error (the facade re-exports it); `OpenOptions` also gets `query_engine: bool` (default true) so the tm-core tier can open on a host without functions/vtab; the M0 minimal-host facade test now sets it false.
- [x] 9.3 Connection tests: a reader query completes while the writer holds a long transaction; a speculative view sees the uncommitted assert and `now()` does not afterwards; a speculative IRI constant matches inside `with` and short-circuits after rollback; speculative terms are absent from the shared cache (spec sql-execution: Connection selection)
- [x] 9.4 Error test: `Unsupported` on a one-reader pool, then a valid query succeeds (spec sql-execution: Typed errors)

## 10. Golden SQL and query-plan tests

- [x] 10.1 Golden SQL `insta` snapshots for the corpus in spec sql-execution "Golden SQL" (single pattern × 6 views, star, chain, layer join, optional, union, filter, aggregate, order/limit, values, virtual predicate, isomorphism, path TVF with a mock operator); includes the no-data-in-text check with `"x' OR 1=1 --"` and byte-identical text across constants and time values
- [x] 10.2 `// @lat: [[tests#Storage Invariants#Views Use Covering Indexes]]`: EXPLAIN QUERY PLAN assertions for Now s/p-bound → `live_spo`, o-bound → `live_osp`, AsOf → `hist_spo`, History p-bound → `hist_pos`, `{Now, At(d)}` p-bound → `live_pos` or `valid_p`, run on a fixture whose predicates have retracted rows so that Now asserts `live_*` (a churn-free predicate may legitimately use `hist_*`, D16), eid constant → `INTEGER PRIMARY KEY`, and `tm:txAdded` constant → `log_add`; also that no case shows `SCAN t0` with a bound position
  - Note: the fixture retracts about 75 % of the rows of each tested predicate; with light churn SQLite chooses hist_spo for a subject+predicate point lookup because the cost ties, so the live_* assertion only holds on a churned predicate (as D16 says).
- [x] 10.3 A golden test that the optional side's `t_ret IS NULL` sits in `ON`, and that every Now alias text contains `tN.t_ret IS NULL` verbatim (spec view-scoped-scans: Verbatim live predicate)
- [x] 10.4 Build the skewed plan fixture in `tm-exec/tests/common`, loaded through the ordinary API only: one class holding 90 % of the nodes, a high-fanout `knows`-like predicate, a 50-row predicate, and churned properties (asserted, retracted, re-asserted); no explicit `optimize()` call, so the test also checks that M0's automatic statistics are present (D16)
  - Note: M0's automatic statistics were not enough: (1) `PRAGMA optimize` analyses with an analysis limit and writes no STAT4 samples, so SQLite could not tell the 50-row predicate from the 18 000-row one and started from the big pattern; (2) pooled readers opened before the first analysis never reloaded statistics (ANALYZE does not bump the schema cookie). Fixed in tm-core `storage/stats.rs`: the trigger points (every `optimize_every` commits, bulk load, open of a populated file without STAT4) run a full `ANALYZE` then the cheap `PRAGMA optimize`, and `refresh_readers` bumps `PRAGMA schema_version` so readers reload the statistics. M0 test helper allow-list gained `sqlite_stat4`. All M0 tests stay green.
- [x] 10.5 `// @lat: [[tests#Query#Skewed Joins Start Selective]]`: golden plan tests with bound parameters on the 10.4 fixture; for every golden BGP (four-pattern star/chain, predicate-only patterns) and every permutation of its IR patterns, the region's query plan starts from the most selective pattern and the results are equal (spec sql-execution: Join order from statistics)
- [x] 10.6 Plan-family test: on the churned predicate a Now pattern inside a join uses a covering `live_*` index; on a churn-free predicate either family is accepted but never a full scan (spec view-scoped-scans: Churned predicate in a join uses the live index, Churn-free predicate may use either family); plus the "Stale statistics keep results" scenario

## 11. Semantic and time-travel tests

- [x] 11.1 Visibility matrix tests: added/retracted boundaries for AsOf, History including retracted rows, cascaded annotations, half-open valid time, unbounded rows, and episodes under `At(d)` (spec view-scoped-scans: Transaction-time visibility, Valid-time visibility)
- [x] 11.2 `// @lat: [[tests#Query#Per Pattern Time Scopes]]`: the IR joins an `AsOf(Tx(150))` pattern and a Now pattern and returns the before/after values of a superseded fact, plus a three-view query
- [x] 11.3 `// @lat: [[tests#Time Travel#Historical Reads Are Stable]]`: a proptest over random assert/create/retract/supersede/cardinality sequences; record AsOf(Tx(t)) IR join results per t and re-check them after all operations
  - Note: the property test does not repeat the `// @lat: [[tests#Time Travel#Historical Reads Are Stable]]` comment because M0's test already carries the single reference for that leaf (one @lat per leaf); the M1 test says so in its module doc.
- [x] 11.4 Snapshot test: a concurrent writer commits between plan and decode (via a test hook in `exec.rs`), and the result equals exactly the before-state or the after-state (spec view-scoped-scans: One snapshot per query)
- [x] 11.5 Virtual predicate behaviour tests: statement parts, variable predicate sees none, virtual wins over a stored `tm:txAdded`, retracted under Now/History/AsOf, absent `txRetracted`/`validFrom`, `validTo` decoding as a datetime with offset `Z`, a `tm:validFrom` constant with another offset matching by instant, `retractKind = 1` after a cascade, wrong-kind object, non-statement subject, and filter on `?t > tx 100`
- [x] 11.6 Decoding tests: one value of every ObjectId tag round-trips through a query, datetimes with their own offset (or none); a TYPED literal keeps its datatype; count/avg decode as integer/double literals; `collect` gives a list
- [x] 11.7 Triangle test: a cyclic BGP returns the brute-force result, and explain reports `CyclicLftjDisabled`; path-pattern test: `Unsupported` with no operator, and correct composition with a mock `tm_path` operator for start-bound and end-bound cases

## 12. Cypher support primitives (cross-change review)

- [x] 12.1 Add `Op::Unnest` to `tm-ir` with lowering to `Values` (constant lists) and `json_each` (computed lists); tests for order preservation and empty/missing lists
- [x] 12.2 Add `Expr::Lookup` (Single / ListIfMany, view-scoped, optional volatile) with correlated-subquery codegen; tests for 0/1/many values and no row multiplication
- [x] 12.3 Add null-safe join variables (`IS` in codegen); test that missing values join only when marked
- [x] 12.4 Add `Op::RowNumber` with `ROW_NUMBER() OVER (PARTITION BY … ORDER BY …)` codegen; test top-1-per-group

## 13. Documentation and wrap-up

- [x] 13.1 Add rustdoc to the public `tm-ir` and `tm-exec` items and the facade methods, and a crate-level example that builds and executes an IR
- [x] 13.2 Update lat.md: in `query.md`, add `@lat` code refs (e.g. `[[crates/tm-exec/src/scan.rs#view_predicates]]` in Views and Scans, `virtual_pred.rs` in Virtual Predicates, `route.rs` in Physical Planning, `sqlgen` in SQL Codegen, and the `tm-ir` `Op` in Logical IR), and `tm-exec/src/host.rs` in `architecture.md#Executor`; add the `InvalidQuery` row to `api.md#Errors`; add new `tests.md` leaves for M1 tests without a spec (e.g. "Unknown Constant Short Circuits", "Set Semantics Dedupes Eids", "Isomorphism Excludes Reused Eids", "Order By Decoded Value") with matching `// @lat:` comments; then run `lat check` until it passes
- [x] 13.3 Run `cargo test --workspace`, `cargo clippy --workspace -D warnings` and `openspec validate add-query-ir-and-sql-planner --strict`, and confirm that every checkbox above is done
