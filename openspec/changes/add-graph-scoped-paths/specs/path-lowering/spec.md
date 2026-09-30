## ADDED Requirements

### Requirement: SPARQL paths inside named graphs
A recursive SPARQL property path inside `GRAPH <g>`, inside `GRAPH ?g`, or in a default graph narrowed by `FROM <g1> FROM <g2>` SHALL be evaluated with graph-scoped evaluation, with the graph set of its block:
- `GRAPH <g>`: the set `[g]`;
- `FROM <g1> … FROM <gn>` (the default graph): the set `[g1, …, gn]`;
- `GRAPH ?g`: when another pattern of the block binds `?g`, the path SHALL use that graph for each of its solutions; otherwise the path SHALL be evaluated once per graph visible in the pattern's view (a graph is visible while a visible membership of a visible statement names it), binding `?g` to that graph.

A zero-length match SHALL follow the rule for a nullable path once per graph in scope: once under `GRAPH <g>` and under a `FROM` default graph, and once per graph `?g` ranges over under `GRAPH ?g`, also when the constant endpoint is in no statement. A `FROM NAMED` list that leaves `<g>` out SHALL still empty a `GRAPH <g>` block. A non-recursive path SHALL keep its translation to triple patterns, each carrying the block's graph selection. A `GRAPH ?g` block whose only pattern is a path SHALL be accepted.

#### Scenario: Path inside GRAPH with a constant graph
- **WHEN** `(:a :knows :b)` and `(:b :knows :c)` are in `<g1>`, `(:c :knows :d)` is in `<g2>`, and `SELECT ?x WHERE { GRAPH <g1> { :a :knows+ ?x } }` is run
- **THEN** the solutions are exactly `:b` and `:c`

#### Scenario: Path inside GRAPH ?g, unbound
- **WHEN** `SELECT ?g ?x WHERE { GRAPH ?g { :a :knows+ ?x } }` is run on the same store
- **THEN** the solutions are exactly `(<g1>, :b)` and `(<g1>, :c)`

#### Scenario: Path inside GRAPH ?g, bound by a triple pattern
- **WHEN** `(:a :type :Person)` is in `<g2>` only, and `SELECT ?g ?x WHERE { GRAPH ?g { :a :type :Person . :a :knows* ?x } }` is run
- **THEN** the only solution is `(<g2>, :a)`

#### Scenario: FROM narrows a path
- **WHEN** `SELECT ?x FROM <g1> FROM <g2> WHERE { :a :knows+ ?x }` is run on the same store
- **THEN** the solutions are exactly `:b`, `:c` and `:d`

#### Scenario: Zero-length match per graph
- **WHEN** `SELECT ?g ?x WHERE { GRAPH ?g { :nobody :knows* ?x } }` is run on a store with the graphs `<g1>` and `<g2>`, and `:nobody` is in no statement
- **THEN** the solutions are exactly `(<g1>, :nobody)` and `(<g2>, :nobody)`, and `SELECT ?x WHERE { GRAPH <g1> { :nobody :knows* ?x } }` gives only `:nobody`

#### Scenario: Path inside GRAPH under asOf
- **WHEN** the membership of `(:b :knows :c)` in `<g1>` was removed in tx 20, and `SELECT ?x WHERE { SERVICE <urn:tiramemsu:tm:asOf/15> { GRAPH <g1> { :a :knows+ ?x } } }` is run
- **THEN** the solutions are `:b` and `:c`, and without the `SERVICE` group the only solution is `:b`
