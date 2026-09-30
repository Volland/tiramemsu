# Parser spike (task 1.2)

Decision 1 of `add-cypher-frontend`: `open-cypher = "=0.2.1"` behind `parse/adapter.rs`.
Result: **kept open-cypher; no switch to the vendored oxilite parser.**

Every query that appears in the `cypher-*` spec scenarios was parsed with
`open_cypher::parse` after the extension pre-pass blanked `USE …`, `REPEATABLE ELEMENTS`
and `DIFFERENT RELATIONSHIPS`. Gaps found, and what we do about each one:

| Gap in open-cypher 0.2.1 | Handling in `tm-cypher` |
|---|---|
| `CALL { … }` subquery clauses are not in its grammar (only procedure calls, and `EXISTS {}` expressions) | `parse/subq.rs` cuts every outermost `CALL [(vars)] { … }` out of the token stream, parses the body recursively over the same byte offsets (all other bytes masked with spaces) and leaves the placeholder procedure call `CALL __tm_sqN()` of the same byte length, so clause order and spans are intact. `CALL { } IN TRANSACTIONS` is `Unsupported`. |
| `FOREACH` and `LOAD CSV` are Neo4j extensions, not openCypher 9 | recognised on a parse failure by their tokens and reported as `Unsupported` |
| schema commands (`CREATE INDEX`, `DROP …`, `SHOW …`) are not in the grammar | recognised by their first words on a parse failure, `Unsupported` |
| `USE`, `REPEATABLE ELEMENTS`, `DIFFERENT RELATIONSHIPS` | the pre-pass (`parse/prepass.rs`) |

Everything else in scope (reads, writes, `WITH`, `UNWIND`, `UNION`, `EXISTS {}`, pattern
predicates, list comprehension, map projection, `CASE`, quantified predicates, `reduce`)
parses natively. Variable-length relationships, `shortestPath`, quantified path patterns,
GQL path modes, `!`/`&`/`%` label expressions and pattern comprehension parse and are
mapped to `Unsupported` by the adapter.

The fallback trigger of Decision 1 (an unfixable parse bug in an in-scope TCK scenario)
has not fired. The TCK run (`tests/tck.rs`) is the continuing evidence.
