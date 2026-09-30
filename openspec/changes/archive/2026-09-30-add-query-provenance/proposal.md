## Why

Agents that answer from memory need to cite the facts they used, and later they need to ask which of their past answers relied on a fact that has since been retracted or superseded. Today a SPARQL answer is a table of values: the caller cannot tell which stored statements produced a row without re-deriving the match by hand, and a statement matched without a reifier (`~ ?r`) leaves no trace in the result at all.

Every stored statement in Tiramemsu has an eid (`lat.md/data-model#Statements`), and the IR already binds eids when a reifier asks for them (`TriplePattern.eid`). So every solution of a SPARQL `SELECT` can carry the set of eids that matched to produce it, at the cost of binding one hidden variable per triple pattern. With those eids stored next to an answer, "is this answer stale?" becomes a lookup of `tm:txRetracted` on a handful of statements.

## What Changes

- **Opt-in provenance on SELECT.** `View::sparql_with(text, &SparqlOptions { provenance: true, .. })` runs a `SELECT` whose `Solutions` carry one sorted, de-duplicated `Vec<Eid>` per row, read with `Solutions::provenance(row)`. `View::sparql(text)` is unchanged and is shorthand for `sparql_with(text, &SparqlOptions::default())`.
- **What counts.** The eid of every stored triple pattern that matched in the solution: the patterns of `OPTIONAL` parts that matched (nothing for an absent part), the `UNION` branch that produced the row, the statements and the `sys:inGraph` membership statements of `GRAPH` blocks, annotation (`{| |}`) and reifier (`~ ?r`) triples, fixed-length property paths (`/`, `|`, `^`), and patterns inside `SERVICE <tm:…>` time scopes (whose eid is visible in that scope's view, even if it is retracted now). Several live eids with the same `(s, p, o)` form one SPARQL triple, so all of them are listed.
- **What does not count.** Statements that are only tested by `FILTER EXISTS`, `FILTER NOT EXISTS` or `MINUS`; virtual predicates (`sys:subject`, `tm:txAdded`, …), which add no eid of their own; and recursive property path regions (`*`, `+`, `?`), because `REACH` evaluation carries no eids (a documented limit of this change).
- **Modifiers, aggregates, subqueries.** With `DISTINCT` (top level), rows equal on the projected variables are merged and their provenance is unioned, and `OFFSET`/`LIMIT` apply after the merge so the row count is exact. A group's provenance is the union over the group's input rows. A subquery's rows carry their provenance into the outer query, including `DISTINCT` and grouped subqueries. `REDUCED` keeps duplicates, each row with its own provenance.
- **Other forms.** Provenance with `ASK`, `CONSTRUCT` or an update request fails with `Unsupported` before any SQL runs. Cypher is out of scope: relationship variables already expose eids.
- **Serialisation.** `write_sparql_json` adds a non-standard top-level member `"provenance"`, an array parallel to `results.bindings` of arrays of statement IRIs (`urn:tiramemsu:stmt:<n>`), only when provenance was requested. Without it the output is byte-identical to today. The JSON bridge `sparql` operation takes an optional `provenance: true` and then returns `"provenance"` beside `"rows"`.

## Capabilities

### New Capabilities
- `query-provenance`: per-solution provenance for SPARQL `SELECT`: the API, what counts and what does not, modifiers, aggregates and subqueries, time scopes and named graphs, the error cases, the serialisations and the stale-answer check.

### Modified Capabilities
- `sparql-query`: "SPARQL JSON results" gains the provenance member, present only when provenance was requested.
- `json-bridge`: "Query results" gains the optional `provenance` argument of `sparql` and the `"provenance"` member of a select result.

## Impact

- **`tm-ir`:** one additive naming convention in `var.rs`: an eid variable whose name starts with `~prov` binds the eid but keeps the set semantics of an unbound eid. No struct or enum changes.
- **`tm-exec`:** the planner keeps the canonical-eid predicate (`lat.md/storage#Multi-Eid Predicates`) for such variables. One condition in `plan/normalize.rs`.
- **`tm-sparql`:** a new `provenance` module (IR instrumentation pass, sibling-eid lookup, row assembly), `Solutions::provenance`, the JSON member and the error-name constants.
- **Facade (`tiramemsu`):** `SparqlOptions` and `View::sparql_with`.
- **`bindings/json`:** the `provenance` argument of `sparql`. The Node and Python bindings pick it up through the bridge without source changes.
- **Not affected:** storage, the ObjectId encoding, Cypher, SPARQL output and plans when provenance is off (golden tests and the W3C subset keep their counts).
- **Docs:** `lat.md/query.md` (SPARQL front end), `lat.md/api.md` and `lat.md/tests.md` when implemented.
