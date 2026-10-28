## 1. Engine plumbing

- [x] 1.1 `DatalogProvider` trait and optional registration on `tm-exec`'s `Engine`; facade implementation over `View::datalog` with `exec` + `datalog`.
- [x] 1.2 IR leaf `DatalogPattern` with display and validation in `crates/tm-ir`.
- [x] 1.3 `tm_datalog` table function and SQL codegen as an unflattened derived table; explain shows one call.
- [x] 1.4 Native operator in `crates/tm-native` evaluating the leaf once per execution.
- [x] 1.5 `ProvenanceGap::DatalogCall`.

## 2. Front ends

- [x] 2.1 SPARQL lowering of `SERVICE <urn:tiramemsu:dl:query>` with `dl:goal`, `dl:program` or `dl:text`, `dl:validFrom`, `dl:validTo`; `DatalogCallShape`.
- [x] 2.2 Cypher procedure `tiramemsu.datalog.query(program, goal)` with constant or parameter arguments; yields goal variables, `validFrom`, `validTo`.
- [x] 2.3 `Unsupported` at execution without the `datalog` feature.
- [x] 2.4 Annotate implementations with `@tg: implements:: [[openspec:datalog-query-calls#<Requirement>]]`.

## 3. Tests

- [x] 3.1 Scenario tests for every scenario of the delta spec on SQLite and redb, annotated `@tg: verifies:: [[openspec:datalog-query-calls#<Requirement>#<Scenario>]]`.
- [ ] 3.2 Differential: random goals through `View::datalog`, SPARQL and Cypher on both backends return equal rows. NOT DONE: `three_surfaces_agree` in `crates/tiramemsu/tests/datalog_calls.rs` covers one fixed temporal goal on both backends; a randomised proptest is still to write.
- [x] 3.3 Feature matrix: `sparql` without `datalog` parses and fails with `Unsupported`.

## 4. Documentation

- [x] 4.1 `lat.md/query.md` section "Datalog Calls" next to Text Recall, with the routes and the once-per-query rule; `lat.md/datalog.md` DL17 implemented with code links; `datalog-query-calls` in the `openspec:` frontmatter of `lat.md/datalog.md`.
- [ ] 4.2 (CHANGELOG done; the recipe is NOT DONE) A recipe in `lat.md/recipes.md` combining a goal call, a pattern and text recall, with its test; CHANGELOG.
- [x] 4.3 `tg check`, `tg ontology check` and `tg trace datalog-query-calls --gaps` show no new gaps; `openspec validate add-datalog-query-calls` passes.
