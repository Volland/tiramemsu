## Context

See `proposal.md` (Why) for the motivation. This section covers the state and constraints that shape the design.

- **Every statement has an eid.** A triple pattern of the IR binds it when `TriplePattern.eid` is set; SPARQL sets it for reifiers (`~ ?r`), triple terms and `rdf:reifies` (`lat.md/query#Front Ends#SPARQL`).
- **Set semantics (D10).** SPARQL runs with `graph_set = SetOfTriples`: a pattern whose eid is *not* bound matches each visible `(s, p, o)` once. The planner enforces this with the canonical-eid predicate `t.eid = (SELECT min(x.eid) … same s, p, o, same view)`, but only for predicates listed in `pred_multi` (`lat.md/storage#Multi-Eid Predicates`) and only while the eid is unbound. Binding the eid naively would therefore change the number of rows.
- **`DISTINCT` over a BGP** skips the canonical predicate altogether (`distinct_over_bgp`), because the outer `DISTINCT` removes duplicates.
- **Named graphs** lower in `tm-exec` (`plan/bind.rs`) to a membership join `(e sys:inGraph g)` under the pattern's view, with an internal eid `~gsel<N>` on the statement pattern (which turns the canonical predicate off for it). Several `FROM` graphs become an `EXISTS`.
- **Internal variables** of the SPARQL lowering start with `~` and are never result columns of `SELECT *`.
- **Parallel work.** Other changes touch the path engine, `PathPattern`, the lowering of paths under `GRAPH`, virtual predicates and the facade view. This change keeps `tm-ir` edits additive and small, and avoids `plan/bind.rs` and `route.rs`.

## Goals / Non-Goals

**Goals:**
- For each `SELECT` solution, the exact set of stored statements whose match produced it, as eids, with no change to the solutions themselves (same rows, same multiplicity, same order under `ORDER BY`).
- Standard output unchanged when provenance is not requested: same plans, same SQL, byte-identical JSON.
- One extra SQL statement at most per distinct view, independent of the number of rows.

**Non-Goals:**
- Provenance of recursive property paths (`*`, `+`, `?`). `REACH` evaluation returns endpoints only.
- Provenance for `ASK`, `CONSTRUCT`, updates and Cypher.
- "Why-not" provenance, or which statements made a `FILTER NOT EXISTS` or `MINUS` succeed.
- Storing answers or provenance: the caller keeps the eids it gets.

## Decisions

### Decision 1: An IR pass, not a new operator

Provenance is computed by rewriting the lowered IR (`tm_sparql::provenance::instrument`), running it on the unchanged executor, and assembling rows in Rust. The IR gets no new operator and no new field.

- Every stored triple pattern gets an eid variable. A pattern whose eid is unbound gets a fresh hidden `~prov<N>`. A pattern that already binds an eid (reifier, triple term) reuses that variable.
- The hidden variables are projected through every `Project` of the tree, so the root returns them as extra columns next to the user's columns. They are never part of `Solutions::vars`.
- A user eid variable (`?r`) that crosses a subquery `Project` is aliased to a fresh `~pa<N>` first, so projecting it cannot leak `?r` into the outer scope.
- `FILTER`, `EXISTS`, `NOT EXISTS`, `MINUS` (lowered to `NOT EXISTS`), `BIND` expressions and `LeftJoin` conditions are not walked, so the statements they test never count.
- Virtual predicates are skipped: they read columns of the subject statement's row, whose own pattern already counts.

*Alternative considered:* a `provenance` flag on `TriplePattern` or a new `Op::Provenance`. Rejected for now: both change public IR types that parallel changes also edit, and nothing in the executor needs more than a bound eid.

### Decision 2: Hidden eids keep set semantics (`~prov` convention)

A `~prov` eid variable binds the eid but plans as if the eid were unbound: under `SetOfTriples` the canonical-eid predicate is kept, so the pattern still matches only `min(eid)` of each visible `(s, p, o)` and the rows are exactly those of the query without provenance. `tm-ir` documents the convention (`tm_ir::var::PROVENANCE_PREFIX`, `is_provenance`) and `tm-exec` checks it in one place (`plan/normalize.rs`).

The canonical eid is only a representative. When several visible eids share the `(s, p, o)`, the solution relied on the SPARQL triple, and every one of them is a statement that says it. After the main query, one sibling lookup per distinct view maps each canonical eid to all visible eids with the same `(s, p, o)`, with the same view predicates the canonical subquery uses:

```
VALUES ?c { … }  (?s ?p ?o) eid ?c view V  (?s ?p ?o) eid ?e view V  → (?c, ?e)
```

The lookup runs in batches of 500 eids and is skipped when no row has a canonical eid.

*Alternative considered:* bind the eid and turn the canonical predicate off, then merge rows that differ only in hidden columns. Rejected: rows that differ only in hidden columns are also produced by genuine bag duplicates (two `UNION` branches, a projection that drops a variable), and those must not merge.

### Decision 3: Named graphs count their membership statements

A statement matched in `GRAPH <g>` or `GRAPH ?g` was chosen because of a membership statement `(e sys:inGraph g)`, and a later retraction of that membership makes the answer stale just as a retraction of `e` does, so its eid counts. The pass rewrites a graph-selected pattern itself, mirroring `plan/bind.rs`:

- `Set([g])` and `Var(g)`: the statement pattern binds `e` (a fresh `~pe<N>` when unbound, a name without the `~prov` prefix, so the canonical predicate stays off exactly as with bind's `~gsel<N>`) and is joined with the membership pattern `(e sys:inGraph g)` whose eid is a `~prov` variable. Both eids count.
- `Set([g1, g2, …])` (several `FROM` graphs): the membership is an `EXISTS` test, as without provenance. Only the statement eid counts. This is documented as a limit.

Annotation triples and virtual predicates have no graph selector already, so nothing changes for them.

### Decision 4: `DISTINCT` is merged in Rust; grouping unions in SQL

- **Top-level `DISTINCT`:** the root `Project{distinct}` becomes a plain projection, `OFFSET`/`LIMIT` above it are removed from the IR, and the rows are merged in Rust on the projected cells, keeping the first occurrence and unioning provenance. `ORDER BY` stays in SQL; its keys use only projected variables, so equal rows sort together and the first occurrence is at the position `DISTINCT` would keep. `OFFSET` and `LIMIT` are then applied to the merged rows, so the count is exact. The cost is that a `DISTINCT … LIMIT` query reads every row before the limit.
- **Aggregates:** for every provenance column in the input of an `Aggregate`, the pass adds `~pg<N> := GROUP_CONCAT(col; separator=" ")` (the codegen renders a statement as its IRI). The group's provenance is the union over its rows. The output column is a list of statement IRIs, parsed back when rows are assembled, and nests: a grouped subquery inside another group concatenates lists.
- **Subqueries:** a non-`DISTINCT` subquery projects its provenance columns (row multiplicity unchanged). A `DISTINCT` subquery becomes `Aggregate(group = projected vars)` with the `GROUP_CONCAT` columns, which is the same relation with provenance unioned per distinct row. `SELECT DISTINCT` of no variable is `Unsupported("provenance with DISTINCT of no variable")` (a group over zero rows would add a row).
- **`REDUCED`:** the lowering already ignores it; rows stay as they are, each with its own provenance.

### Decision 5: Opt in per call, fail on forms without rows

`SparqlOptions` is a plain struct with `Default` (provenance off), so new options can be added with `..Default::default()`. With provenance on, `ASK`, `CONSTRUCT` and updates fail with `Unsupported` naming `"provenance for ASK"`, `"provenance for CONSTRUCT"` and `"provenance for updates"` before any SQL runs. `ASK` has no rows to annotate; a caller who wants the witnesses runs the same pattern as `SELECT * … LIMIT 1`.

### Decision 6: Wire formats

- `Solutions` gains `provenance: Option<Vec<Vec<Eid>>>`, `None` unless requested, and `Solutions::provenance(row) -> Option<&[Eid]>`.
- SPARQL JSON: a top-level `"provenance"` array between `"head"` and `"results"`, one array of `urn:tiramemsu:stmt:<n>` strings per binding, only when `Solutions::provenance` is `Some`. Clients that follow the SPARQL 1.1 JSON format ignore unknown top-level members. It must precede `"results"`: the streaming parser of `sparesults` (Oxigraph) stops at the end of the bindings and rejects any member after them.
- JSON bridge: `{"kind": "select", "vars", "rows", "provenance": [[{"stmt": n}, …], …]}`, using the bridge's own encoding of statements.

### Decision 7: Cypher is out of scope

Cypher relationship variables are eids already (`lat.md/query#Front Ends#Cypher Dual View`), and a caller can `RETURN r` for every relationship it wants to cite. Property reads (`n.name`) would need the same pass over the Cypher interpreter, which runs projections and aggregates in Rust. Left for a follow-up if asked for.

## Risks / Trade-offs

- **Extra columns change the SQL.** Binding the eid of every pattern adds output columns and removes the `DISTINCT`-over-BGP shortcut, so a provenance query may be slower than the same query without it. Provenance is opt-in and the default path is untouched.
- **`DISTINCT … LIMIT` reads every row.** Exactness is worth more than the early stop here; documented.
- **`GROUP_CONCAT` strings grow with the group.** A group of 100 000 rows yields a long string. Acceptable for the agent-memory workloads this targets.
- **Naming convention in `tm-ir`.** A prefix check is weaker than a typed flag. It is confined to one function (`is_provenance`) and one call site, and can be replaced by a field when the IR is next changed on purpose.

## Migration Plan

Additive: no stored data, format version or default behaviour changes.

## Open Questions

- Should recursive paths report the eids of one witness path? That needs the path engine's `Trail` mode inside SPARQL and is left for the path-engine follow-up.
- Should `ASK` return the witnesses of its one solution? Deferred until a caller needs it.
