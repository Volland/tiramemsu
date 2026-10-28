# Datalog goals inside SPARQL and Cypher

Source: decision DL17 of `lat.md/datalog.md` (interview of 2026-10-28). Change 4 of four; depends on `add-datalog-engine`.

## Why

A recursive or temporal question often belongs inside a larger query: "the people alice reports to (Datalog), who are also mentioned in notes about Lisbon (text recall), with their current email (pattern)". Without a call from SPARQL or Cypher, an agent has to run the goal separately and splice values into a second query. A goal with constant arguments is a bounded table that magic sets compute cheaply, so it fits a query as a table call, unlike a whole-scope reasoner ([[reasoning#Every Surface]]).

## What Changes

- **SPARQL:** `SERVICE <urn:tiramemsu:dl:query> { [] dl:program <iri> | dl:text "…" ; dl:goal "reports(ex:alice, X)" ; dl:validFrom ?f ; dl:validTo ?t }` binds the goal's variables to the SPARQL variables of the same name (`X` → `?X`), one row per goal row and interval fragment; `dl:validFrom` and `dl:validTo` are optional.
- **Cypher:** `CALL tiramemsu.datalog.query(program, goal) YIELD X, validFrom, validTo`, with `program` a stored program's IRI or `{text: "…"}`.
- **Evaluated once:** the program and goal must be constants; the call runs once per query execution, never per row, in the query's snapshot and under its budget, and its rows join with the rest of the query.
- **Two routes:** a `tm_datalog` table function on SQLite and a native operator on redb, both over one IR leaf `DatalogPattern`, returning identical rows.
- **Layering:** `tm-exec` and `tm-native` call Datalog through a provider trait the facade registers when its features `exec` and `datalog` are both enabled; without `datalog` the call fails with `Unsupported`, and the front ends still parse it.
- **Provenance:** a query with provenance on records a `ProvenanceGap::DatalogCall` for rows from the call.

## Capabilities

### New Capabilities

- `datalog-query-calls`: the SPARQL service and Cypher procedure for Datalog goals, their evaluation once per query, variable binding, budgets, routes and feature gating.

### Modified Capabilities

None.

## Impact

- **`crates/tm-ir`:** IR leaf `DatalogPattern { program, goal, outputs, valid_vars }`.
- **`crates/tm-sparql`, `crates/tm-cypher`:** lowering of the service and the procedure.
- **`crates/tm-exec`:** provider trait `DatalogProvider`, the `tm_datalog` table function, planner routing (the call is materialised once, like `tm_lftj`).
- **`crates/tm-native`:** a native operator for the leaf.
- **`crates/tiramemsu`:** provider registration with `exec` + `datalog`.
- **lat.md:** `lat.md/query.md` (a section for Datalog calls next to Text Recall), `lat.md/datalog.md` (DL17 implemented), `lat.md/recipes.md` (a mixed recipe with its test).
- **Binding decisions and constraints:** [[query#Query Budgets]], [[query#Front Ends]], [[architecture#Crates]] (front ends depend only on the IR), [[constraints#Same Logical Database]].
- **Requirements added:** [[openspec:datalog-query-calls#SPARQL goal service]], [[openspec:datalog-query-calls#Cypher goal procedure]], [[openspec:datalog-query-calls#Evaluated once per query]].

## Non-goals

- No correlated calls (goal arguments from the outer query's rows).
- No derived relations as virtual predicates.
- No writes from the call.
