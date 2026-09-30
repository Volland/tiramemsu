## Dependencies

- M1 (`add-query-ir-and-sql-planner`), M2a (`add-sparql-frontend`) and `add-named-graphs` are archived and are the base of this change.
- Parallel changes edit the path engine, `PathPattern`, the lowering of paths under `GRAPH`, virtual predicates and the facade view. This change keeps `tm-ir` edits additive (a constant and a function in `var.rs`) and does not touch `plan/bind.rs`, `plan/route.rs` or `tm-exec/src/path`.
- Test code references lat.md test specs with `// @lat: [[tests#…]]`, placed next to the covering test. The requirement text is in `specs/*/spec.md` of this change.

## 1. IR convention and planner

- [x] 1.1 Add `tm_ir::var::PROVENANCE_PREFIX` (`"~prov"`) and `is_provenance(&Var)`, documented as "binds the eid, keeps the set semantics of an unbound eid". Unit-test the predicate.
- [x] 1.2 In `tm-exec` `plan/normalize.rs`, keep the canonical-eid predicate for a triple pattern whose eid is a provenance variable. Add a planner test that a `~prov` eid on a `pred_multi` predicate still gets the canonical predicate and a user eid does not.

> Notes (group 1): `tm_ir::var::PROVENANCE_PREFIX` and `is_provenance` are the only `tm-ir` change (additive, with a doctest). The planner change is one condition in `plan/normalize.rs`: `canonical` now holds when the eid is `None` *or* a provenance variable. `plan/bind.rs` is untouched. Test `provenance_eid_keeps_set_semantics` in `crates/tm-exec/tests/sql_exec.rs`: with two `create`d parallel edges, a `~prov0` eid gives one row bound to the minimum eid and the SQL keeps `min(x0.eid)`, and a user `?prov0` eid gives two rows.

## 2. Instrumentation pass (`tm-sparql`)

- [x] 2.1 Add `tm_sparql::provenance` with `instrument(&QueryPlan) -> Result<ProvenancePlan>`: fail with `Unsupported` for `ASK` and `CONSTRUCT`; give every stored triple pattern an eid variable (`~prov<N>` when unbound, the existing variable otherwise); skip virtual predicates; do not walk expressions (`FILTER`, `EXISTS`, `MINUS`, `BIND`, `LeftJoin` conditions).
- [x] 2.2 Rewrite graph-selected patterns: `Set([g])` and `Var(g)` become the statement pattern (eid `~pe<N>` when unbound) joined with `(e sys:inGraph g)` whose eid is a `~prov` variable; `Set` of several keeps its selector and counts the statement eid only.
- [x] 2.3 Project provenance columns through every `Project`; alias user eid variables to `~pa<N>` below a subquery `Project`; turn each provenance column entering an `Aggregate` into a `GROUP_CONCAT` column; turn a `DISTINCT` subquery into an `Aggregate` over its projected variables (`Unsupported("provenance with DISTINCT of no variable")` for none).
- [x] 2.4 At the root, drop `DISTINCT` and the `OFFSET`/`LIMIT` above it from the IR and record them in the plan; keep `ORDER BY`.
- [x] 2.5 Add the sibling lookup (`ProvenancePlan::sibling_queries`, batches of 500) and row assembly (`ProvenancePlan::assemble`): expand canonical eids, parse `GROUP_CONCAT` lists, merge `DISTINCT` rows in first-occurrence order, apply `OFFSET`/`LIMIT`, and sort and de-duplicate each row's eids.
- [x] 2.6 Unit tests of the pass on lowered IR: hidden variables, graph rewrite, aggregate columns, root modifiers, the `Unsupported` cases; and of assembly on hand-built rows.

> Notes (group 2): `crates/tm-sparql/src/provenance.rs`. `instrument(&QueryPlan) -> Result<ProvenancePlan>` leaves `QueryPlan` and `prepare` unchanged, and `ProvenancePlan` carries the rewritten `IrQuery`, the visible variables (`output_vars` of the original root), the hidden columns (`ProvColumn { var, view, siblings, list }`) and the top-level `distinct`/`skip`/`limit`. Hidden names are `~prov<N>` (canonical-preserving eids), `~pe<N>` (graph statement eids, non-canonical like bind's `~gsel`), `~pa<N>` (aliases of user eid variables leaving a subquery) and `~pg<N>` (group lists). None of them collide with the lowering's `~p<digits>` path variables. `assemble` takes a `run` closure for the sibling lookups, so `tm-sparql` still does not depend on `tm-exec`. Group lists use `GROUP_CONCAT(col; separator=" ")`: the codegen renders a statement as `urn:tiramemsu:stmt:<n>`, and nesting concatenates lists (checked with a probe before choosing this over `collect`). Rows are merged on the `Debug` text of the projected cells, which matches SQL `DISTINCT` on term ids because equal ids decode to equal values. Unit tests: `hidden_columns_per_pattern`, `graph_patterns_bind_the_membership`, `modifiers_and_groups`, `unsupported_forms`, `assemble_rows`.

## 3. Results and facade

- [x] 3.1 Add `Solutions::provenance` (field and accessor) and the `"provenance"` member of `json::write_select`, present only when provenance is set. Test that a document without provenance is unchanged and one with it parses back with `sparesults`.
- [x] 3.2 Add `SparqlOptions` (with `Default`) and `View::sparql_with` to the facade, with `View::sparql` as the shorthand; run the sibling lookups on the same view source; reject updates with `"provenance for updates"` before the transaction opens. Doctest the new API.
- [x] 3.3 JSON bridge: accept `provenance` on `sparql` and add `"provenance"` to a select result. Test it.

> Notes (group 3): `Solutions` gains the public field `provenance: Option<Vec<Vec<Eid>>>`. The four struct-literal sites in the repo (the `Solutions` doctest, the `json.rs` tests and the facade's `to_solutions`) now set it. The JSON member is written **between `head` and `results`**, not after them as first drafted: `sparesults`' streaming parser stops at the end of the bindings and rejects any later member, while it skips unknown members before `results`. The spec and design were updated to match. `SparqlOptions` is a plain `Default` struct re-exported from `tiramemsu`. Clippy's `needless_update` rejects `..Default::default()` on a one-field struct in linted code, so only the doc example uses it. `run_provenance` runs the main query and every sibling lookup inside one `View::exec` closure, so a now view cannot change between them. It uses the same cache mode as `execute_ir` and does not touch `view.rs`. Updates with provenance are rejected before the transaction opens. Bridge: `provenance` must be a boolean or null (`InvalidArgument` otherwise), and statements are encoded as `{"stmt": n}` like every other bridge value.

## 4. Behaviour tests (facade)

- [x] 4.1 Basic BGP, `OPTIONAL` present and absent, `UNION` branches, annotations and reifiers, `FILTER EXISTS` and `MINUS` not counted, virtual predicates add nothing, fixed-length path triples counted and recursive paths not.
- [x] 4.2 `DISTINCT` merge, `LIMIT`/`OFFSET` after `DISTINCT`, `ORDER BY` kept, aggregates (with and without `GROUP BY`, over zero rows), subqueries (plain, `DISTINCT`, grouped).
- [x] 4.3 Multi-eid predicate: one row listing both eids; a reifier still gives one row per eid.
- [x] 4.4 `GRAPH <g>` and `GRAPH ?g` list the membership eid; several `FROM` graphs list the statement only; `SERVICE <tm:asOf/N>` returns a now-retracted eid.
- [x] 4.5 Off by default: `sparql` and `sparql_with` with provenance off give equal results and byte-identical JSON; `ASK`, `CONSTRUCT` and updates with provenance fail with `Unsupported` and write nothing.
- [x] 4.6 The stale-answer recipe: keep a row's eids, retract one, detect it through the current view and `tm:txRetracted` on the history view.
- [x] 4.7 Property test: for random small data (with duplicate episodes) and random BGPs, provenance never changes the solutions, every provenance eid is visible in the view, and running the query on a fresh store holding only the provenance statements returns the row.

> Notes (group 4): `crates/tiramemsu/tests/sparql_provenance.rs`. Every check runs the query with and without provenance and asserts the same variables and rows (as a multiset, or in order under `ORDER BY`). There is also a corpus test over every `SELECT` of the differential corpus (5 files, 40+ queries across 9 fixtures, including the `parallel` multi-eid fixture), which checks unchanged rows and that cited eids are visible in the view (skipped for texts with `FROM`/`SERVICE` time clauses, which read another view). The property test (`provenance_is_sound`, 40 cases) generates up to 12 statements, each optionally duplicated with `create`, and 1–3-pattern BGPs over 4 nodes, 3 predicates and 3 variables. It checks unchanged rows, that cited eids are visible, and that a fresh store with only the cited `(s, p, o)` reproduces each row. Findings while testing: `?e sys:inGraph <g> ~ ?m` cites just the membership (a membership read as an ordinary statement), and a recursive path joined with a triple cites only the triple.

## 5. Documentation and checks

- [x] 5.1 Update `lat.md/query.md` (SPARQL front end: provenance bullet and limits), `lat.md/api.md` (`sparql_with`, `SparqlOptions`, `Solutions::provenance`) and add `lat.md/tests.md` sections referenced from the tests. Run `lat check`.
- [x] 5.2 `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, and `cargo test -p tm-sparql --test w3c` with unchanged counts.

> Notes (group 5): `lat.md/query.md` gains `#### Query Provenance` under `### SPARQL`, which links the code (`instrument`, `ProvenancePlan#assemble`, `is_provenance`). `lat.md/api.md` gains `sparql_with` in the signature block plus a bullet. `lat.md/tests.md` gains `## Query Provenance` with 15 leaves, each referenced exactly once (facade tests, the corpus and property tests, the `tm-exec` planner test, two `tm-sparql` unit tests and the bridge test). There is no new decision row in `overview.md`, as agreed. Results: `lat check` clean, fmt and clippy clean, `cargo test --workspace` 1 029 passed and 0 failed across 109 suites (doctests included), W3C unchanged at 634 passed and 147 failed (147 expected). This change is not archived.
