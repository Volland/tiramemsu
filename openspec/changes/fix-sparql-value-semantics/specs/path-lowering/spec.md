## REMOVED Requirements

### Requirement: Unsupported SPARQL path forms
**Reason**: Negated property sets are now supported; the requirement "SPARQL negated property sets" replaces it.
**Migration**: Queries that caught the `Unsupported("negated property sets")` error can use `!p`, `!(p|q)`, `!^p` and `!(p|^q)` directly.

## ADDED Requirements

### Requirement: SPARQL negated property sets
A negated property set `!p`, `!(p1|…|pn)`, `!^p` or a mixed set `!(p|^q)` SHALL match the statements in the pattern's view, and graph scope, that a variable-predicate pattern in the same direction would match, minus those whose predicate is in the set; forward members match subject to object and inverse members object to subject. Virtual predicates SHALL never be matched by a negated set. A negated set SHALL be usable as one step and inside `*`, `+`, `?`, sequences and alternatives, with the same results as the equivalent filtered patterns.

#### Scenario: One-step negation
- **WHEN** `(v:a v:knows v:b)` and `(v:a v:likes v:c)` are live and `SELECT ?x WHERE { v:a !v:knows ?x }` runs
- **THEN** the only row is `?x = v:c`

#### Scenario: Inverse member
- **WHEN** `(v:b v:knows v:a)` and `(v:c v:likes v:a)` are live and `SELECT ?x WHERE { v:a !^v:knows ?x }` runs
- **THEN** the only row is `?x = v:c`

#### Scenario: Negation inside a recursive path
- **WHEN** `(v:a v:next v:b)`, `(v:b v:next v:c)` and `(v:b v:blocked v:d)` are live and `SELECT ?x WHERE { v:a (!v:blocked)+ ?x }` runs
- **THEN** the rows are `v:b` and `v:c`, and not `v:d`

#### Scenario: Negation honors the view
- **WHEN** `(v:a v:likes v:c)` was retracted in transaction 5 and `SELECT ?x WHERE { SERVICE <urn:tiramemsu:tm:asOf/4> { v:a !v:knows ?x } }` runs
- **THEN** `v:c` is returned, and it is not returned by the same query without the time scope
