# query-provenance Specification

## Purpose
TBD - created by archiving change add-query-provenance. Update Purpose after archive.

## Requirements

### Requirement: Provenance is requested per query

The facade SHALL offer `View::sparql_with(text, &SparqlOptions)`, where `SparqlOptions` implements `Default` with provenance off, and `View::sparql(text)` SHALL behave exactly as `sparql_with(text, &SparqlOptions::default())`. With `provenance: true`, a `SELECT` SHALL return `Solutions` whose `provenance` is present with one entry per row, and `Solutions::provenance(row)` SHALL return that row's eids, sorted ascending and without duplicates. Without it, `Solutions::provenance(row)` SHALL return `None` for every row, and the solutions, their order and every serialisation SHALL be identical to those of the same query run without the option.

#### Scenario: Basic graph pattern
- **WHEN** `e1 = (v:alice v:worksAt v:acme)` and `e2 = (v:acme v:locatedIn v:paris)` are live, and `SELECT ?c WHERE { v:alice v:worksAt ?o . ?o v:locatedIn ?c }` runs with provenance
- **THEN** one row `c = v:paris` is returned and its provenance is `[e1, e2]`

#### Scenario: Off by default
- **WHEN** the same query runs with `View::sparql`
- **THEN** the row is the same and `Solutions::provenance(0)` is `None`

#### Scenario: Solutions do not change
- **WHEN** any `SELECT` of the test corpus runs with and without provenance on the same view
- **THEN** both return the same variables and the same rows in the same order (as a multiset when the query has no `ORDER BY`)

### Requirement: Matched statements count

The provenance of a row SHALL contain the eid of every stored statement matched by a triple pattern that produced the row: the patterns of the group, the patterns of an `OPTIONAL` part that matched (and none of an `OPTIONAL` part that did not), the patterns of the `UNION` branch that produced the row, reifier and triple-term patterns, annotation (`{| … |}`) triples, and the triple patterns of fixed-length property paths (`/`, `|`, `^`). Statements matched only inside `FILTER EXISTS`, `FILTER NOT EXISTS` or `MINUS` SHALL NOT be listed. Virtual predicates (`sys:subject`, `sys:predicate`, `sys:object`, `tm:txAdded`, `tm:txRetracted`, `tm:validFrom`, `tm:validTo`, `tm:retractKind`) SHALL add no eid. Recursive property path patterns (`*`, `+`, `?`) SHALL add no eid in this version.

#### Scenario: OPTIONAL present and absent
- **WHEN** `e1 = (v:bob v:name "Bob")`, `e2 = (v:bob v:age 41)` and `e3 = (v:carol v:name "Carol")` are live, and `SELECT ?p ?a WHERE { ?p v:name ?n OPTIONAL { ?p v:age ?a } }` runs with provenance
- **THEN** the row of `v:bob` has provenance `[e1, e2]` and the row of `v:carol` has provenance `[e3]`

#### Scenario: UNION counts the branch taken
- **WHEN** `e1 = (v:a v:p v:x)` and `e2 = (v:a v:q v:y)` are live, and `SELECT ?o WHERE { { v:a v:p ?o } UNION { v:a v:q ?o } }` runs with provenance
- **THEN** the row `o = v:x` has provenance `[e1]` and the row `o = v:y` has provenance `[e2]`

#### Scenario: Annotations and reifiers
- **WHEN** `e1 = (v:alice v:worksAt v:acme)` and `e2 = (e1 v:source v:crawler)` are live, and `SELECT ?s WHERE { v:alice v:worksAt v:acme {| v:source ?s |} }` runs with provenance
- **THEN** the row `s = v:crawler` has provenance `[e1, e2]`

#### Scenario: FILTER EXISTS is not counted
- **WHEN** `e1 = (v:a v:p v:x)` and `e2 = (v:x v:q v:y)` are live, and `SELECT ?o WHERE { v:a v:p ?o FILTER EXISTS { ?o v:q ?z } }` runs with provenance
- **THEN** the row `o = v:x` has provenance `[e1]`

#### Scenario: Virtual predicates add nothing
- **WHEN** `SELECT ?t WHERE { v:a v:p v:x ~ ?r . ?r tm:txAdded ?t }` runs with provenance on the data above
- **THEN** the row's provenance is `[e1]`

### Requirement: One SPARQL triple lists all its eids

Under set semantics, several visible eids with the same `(s, p, o)` are one SPARQL triple, and a pattern whose eid is not bound matches it once. When such a pattern produced a row, the provenance SHALL contain every eid with that `(s, p, o)` that is visible in the pattern's view, and the rows SHALL be those of the query without provenance. A pattern that binds the eid (a reifier) SHALL still produce one row per eid, each listing its own eid.

#### Scenario: Two episodes, one row, both eids
- **WHEN** `(v:alice v:worksAt v:acme)` is live twice as `e1` and `e2` (two valid-time episodes), and `SELECT ?c WHERE { v:alice v:worksAt ?c }` runs with provenance
- **THEN** exactly one row is returned, with provenance `[e1, e2]`

#### Scenario: Reifier keeps one row per eid
- **WHEN** the same data is queried with `SELECT ?r WHERE { v:alice v:worksAt v:acme ~ ?r }` and provenance
- **THEN** two rows are returned, with provenance `[e1]` and `[e2]`

### Requirement: Modifiers, aggregates and subqueries

With `DISTINCT`, rows equal on the projected variables SHALL be merged into the first of them and their provenance unioned, and `OFFSET` and `LIMIT` SHALL apply to the merged rows, so the rows returned are those of the query without provenance. `ORDER BY` SHALL order the rows as without provenance. `REDUCED` MAY keep duplicates; each row then carries its own provenance. The provenance of a group SHALL be the union of the provenance of the group's input rows (an empty list for the group of an aggregate over no rows). A subquery's rows SHALL carry their provenance into the outer query, which unions it with that of its own patterns; a `DISTINCT` subquery unions the provenance of the rows it merges.

#### Scenario: DISTINCT merges provenance
- **WHEN** `e1 = (v:alice v:knows v:bob)` and `e2 = (v:alice v:knows v:carol)` are live, and `SELECT DISTINCT ?s WHERE { ?s v:knows ?o }` runs with provenance
- **THEN** one row `s = v:alice` is returned with provenance `[e1, e2]`

#### Scenario: LIMIT after DISTINCT stays exact
- **WHEN** `v:alice` and `v:bob` each know two people, and `SELECT DISTINCT ?s WHERE { ?s v:knows ?o } ORDER BY ?s LIMIT 1` runs with provenance
- **THEN** exactly one row `s = v:alice` is returned, with the eids of both of alice's statements

#### Scenario: Group provenance
- **WHEN** the same data is queried with `SELECT ?s (COUNT(*) AS ?n) WHERE { ?s v:knows ?o } GROUP BY ?s` and provenance
- **THEN** each row lists the two eids of its subject's statements

#### Scenario: Subquery provenance reaches the outer row
- **WHEN** `SELECT ?s ?c WHERE { ?s v:worksAt ?c { SELECT ?s WHERE { ?s v:name ?n } } }` runs with provenance
- **THEN** each row lists the eid of its `v:worksAt` statement and the eid of its `v:name` statement

### Requirement: Time scopes and graphs

A pattern inside `SERVICE <urn:tiramemsu:tm:…>` SHALL contribute the eid it matched in that scope's view, even when the statement is retracted in the view the query was submitted on. A pattern inside `GRAPH <g>` or `GRAPH ?g` SHALL contribute its statement's eid and the eid of the `sys:inGraph` membership statement that selected it. With several `FROM` graphs as the default graph, a pattern SHALL contribute its statement's eid only, because its membership is an existence test.

#### Scenario: Retracted statement cited from the past
- **WHEN** `e1 = (v:alice v:worksAt v:acme)` was asserted in tx 1 and retracted in tx 2, and `SELECT ?c WHERE { SERVICE <urn:tiramemsu:tm:asOf/1> { v:alice v:worksAt ?c } }` runs with provenance on the current view
- **THEN** one row `c = v:acme` is returned with provenance `[e1]`

#### Scenario: GRAPH lists the membership
- **WHEN** `e1 = (v:alice v:worksAt v:acme)` is a member of `<g1>` through the membership statement `m1`, and `SELECT ?c WHERE { GRAPH <g1> { v:alice v:worksAt ?c } }` runs with provenance
- **THEN** the row's provenance is `[e1, m1]`

### Requirement: Stale answers can be detected

The eids of a row's provenance SHALL be statement eids that can be looked up later: after one of them is retracted, the statement SHALL no longer be visible in the current view, and a history view SHALL report its `tm:txRetracted`.

#### Scenario: Detect a stale answer
- **WHEN** an answer's provenance `[e1, e2]` is stored, `e1` is retracted in tx 5, and `SELECT ?e ?t WHERE { VALUES ?e { <urn:tiramemsu:stmt:e1> <urn:tiramemsu:stmt:e2> } ?e tm:txRetracted ?t }` runs on the history view
- **THEN** one row `e = e1, t = 5` is returned, and `e1` is not among the current view's statements

### Requirement: Unsupported combinations fail before execution

With provenance requested, `ASK` SHALL fail with `Unsupported { feature: "provenance for ASK" }`, `CONSTRUCT` with `"provenance for CONSTRUCT"`, an update request with `"provenance for updates"`, and `SELECT DISTINCT` of no variable inside a subquery with `"provenance with DISTINCT of no variable"`, all before any SQL is executed and before any write.

#### Scenario: ASK with provenance
- **WHEN** `ASK { ?s ?p ?o }` runs with provenance
- **THEN** it fails with `Unsupported { feature: "provenance for ASK" }`

#### Scenario: Update with provenance
- **WHEN** `INSERT DATA { v:a v:p v:b }` runs with provenance
- **THEN** it fails with `Unsupported { feature: "provenance for updates" }` and nothing is written

### Requirement: Provenance in SPARQL JSON

When a `SELECT` result carries provenance, its SPARQL JSON document SHALL have a top-level member `"provenance"` between `"head"` and `"results"`: an array parallel to `results.bindings` whose entries are arrays of the statement IRIs `urn:tiramemsu:stmt:<n>` of the row, ascending. The member SHALL be absent when provenance was not requested.

#### Scenario: JSON member
- **WHEN** a one-row result with provenance `[e5, e7]` is serialised
- **THEN** the document has `"provenance":[["urn:tiramemsu:stmt:5","urn:tiramemsu:stmt:7"]]` between `head` and `results`, is otherwise byte-identical to the document without provenance, and still parses as SPARQL 1.1 JSON results
