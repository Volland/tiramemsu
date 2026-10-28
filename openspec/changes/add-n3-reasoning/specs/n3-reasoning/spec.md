## ADDED Requirements

### Requirement: Reason over a view
With the facade feature `n3`, `View::reason(rules, &ReasonOptions)` SHALL run the N3 rules (`Rules::Text(n3)` or `Rules::Stored(rule_set)`) over statements of the view and return a `ReasonResult` with the conclusions that are not among the loaded statements, the view's transaction number, `now`, the number of loaded statements, whether pruning applied, the number of segments and a completion status. It SHALL write nothing. A parse error SHALL fail with `N3Parse { line, message }`.

#### Scenario: Socrates
- **WHEN** `(v:socrates rdf:type v:Human)` is live and `reason` runs `{ ?x a v:Human } => { ?x a v:Mortal }.`
- **THEN** the result holds exactly one conclusion `(v:socrates rdf:type v:Mortal)` and the store is unchanged

#### Scenario: Parse error
- **WHEN** `reason` runs the text `{ ?x a v:Human } => `
- **THEN** it fails with `N3Parse` and a line number

### Requirement: Reasoning scope and pruning
A run SHALL load the statements of its scope: the whole view (default), the members of the given named graphs, or the statements whose subject or object lies within `hops` of the seed nodes. Within the scope it SHALL load only statements whose predicate occurs in a predicate position of some rule body, including backward rules and nested formulas, unless a rule body has a variable predicate, in which case it SHALL load the whole scope. The result SHALL report `loaded` and `pruned`.

#### Scenario: Pruning skips unused predicates
- **WHEN** the view holds 100 `v:name` statements and 3 `rdf:type v:Human` statements and the Socrates rule runs
- **THEN** `loaded` is 3 and `pruned` is true

#### Scenario: Variable predicate loads everything
- **WHEN** a rule body is `{ ?s ?p ?o }`
- **THEN** every statement of the scope is loaded and `pruned` is false

#### Scenario: Graph scope
- **WHEN** `(v:a rdf:type v:Human)` is a member of `v:g1` and `(v:b rdf:type v:Human)` is not, and the scope is `Graphs([v:g1])`
- **THEN** only `v:a` is concluded to be mortal

### Requirement: Pointwise valid time
A conclusion SHALL hold at instant `t` if and only if it follows from the loaded statements valid at `t`. For a view with `valid_at(t)` the run SHALL reason once over the statements valid at `t`, and every conclusion's interval SHALL be `[t, t+1)`. Otherwise the run SHALL cut its window (default unbounded) at every valid-time bound of the loaded statements inside it, reason once per segment over the statements valid throughout the segment, and report each conclusion with the union of its segments, merging adjacent segments only when content and support are equal. `TimeMode::Atemporal` SHALL ignore valid time and report unbounded conclusions.

#### Scenario: Facts never simultaneously valid do not combine
- **WHEN** `(v:alice v:worksAt v:acme)` is valid `[2020, 2022)`, `(v:acme v:locatedIn v:berlin)` is valid `[2024, ∞)` and the rule `{ ?p v:worksAt ?c. ?c v:locatedIn ?l } => { ?p v:worksIn ?l }` runs
- **THEN** there is no conclusion

#### Scenario: Overlap gives the intersection
- **WHEN** the same facts are valid `[2020, 2025)` and `[2024, ∞)`
- **THEN** the only conclusion is `(v:alice v:worksIn v:berlin)` with interval `[2024, 2025)`

#### Scenario: Snapshot view
- **WHEN** the view is `valid_at(2024-06-01)` over the facts of the previous scenario
- **THEN** the conclusion holds with interval `[2024-06-01, 2024-06-01 + 1 ms)`

#### Scenario: Negation is evaluated per segment
- **WHEN** `(v:bob rdf:type v:Employee)` is valid always, `(v:bob v:manager v:carol)` is valid `[2023, ∞)` and the rule `{ ?x a v:Employee. ?SCOPE log:notIncludes { ?x v:manager ?m } } => { ?x a v:Unmanaged }` runs
- **THEN** `(v:bob rdf:type v:Unmanaged)` is concluded with interval `(−∞, 2023)` only

#### Scenario: Segment limit
- **WHEN** the loaded statements produce more than `max_segments` segments
- **THEN** the result status is `Incomplete` with limit `Segments`, and every reported conclusion is sound for the segments that ran

### Requirement: Transaction time and the pinned clock
A run SHALL reason over the statements visible in the view's transaction time, so an `as_of(t)` view reasons over memory as believed at `t`. `now` SHALL be the view's instant (the as-of instant, or one instant taken when a run on a now view starts), constant for the whole run and reported in the result. `time:localTime` and the virtual triple `tm:run tm:now ?t` SHALL yield `now` as `xsd:dateTime`.

#### Scenario: As-of reasoning ignores later facts
- **WHEN** `(v:socrates rdf:type v:Human)` is asserted in transaction 2 and `reason` runs on `as_of(Tx(1))`
- **THEN** there is no conclusion

#### Scenario: Reproducible clock
- **WHEN** a rule binds `"" time:localTime ?t` and concludes `(v:run v:at ?t)`, and it runs twice on `as_of(Tx(3))`
- **THEN** both runs conclude the same `?t`, equal to the instant of transaction 3

### Requirement: Statements in N3
Values SHALL map to N3 terms as the RDF rendering of fact bundles does: IRIs and literals as themselves, anonymous nodes as `urn:tiramemsu:node:N`, blank nodes as `urn:tiramemsu:bnode:N` and statements as `urn:tiramemsu:stmt:N`, and back. For each loaded statement `e` the virtual triples `e tm:content { s p o }`, `e tm:validFrom dt` and `e tm:validTo dt` (bounded ends only) SHALL be available to rules when a rule body uses that predicate or a variable predicate; they SHALL NOT be stored or reported as conclusions.

#### Scenario: Layer on a statement
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` has the layer `(e1 v:confidence 0.9)` and the rule `{ ?e v:confidence ?c. ?c math:greaterThan 0.8. ?e tm:content { ?s ?p ?o } } => { ?s v:trusted ?o }` runs
- **THEN** `(v:alice v:trusted v:acme)` is concluded with premises `[e1's layer, e1]`

#### Scenario: Validity as data
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` is valid `[2020-01-01, 2023-01-01)` and a rule computes tenure from `?e tm:validFrom ?a. ?e tm:validTo ?b`
- **THEN** the rule sees `?a = "2020-01-01T00:00:00Z"^^xsd:dateTime` and `?b = "2023-01-01T00:00:00Z"^^xsd:dateTime`

### Requirement: Proofs and completion
Every conclusion SHALL carry a proof `{ rule_set, rule, premises }`, where `rule` is the rule's index in its rule set and `premises` lists the eids of loaded statements and the indexes of other conclusions it used. With `proof_n3: true` the result SHALL also carry eyeron's N3 proof text. The status SHALL be `Complete` or `Incomplete { limits }` naming every limit reached (`Segments`, `Iterations`, `MatchSteps`, `BackwardDepth`, `TermBytes`). A fired inference fuse SHALL be reported as `contradiction` with the rule and its instance.

#### Scenario: Proof lists premise eids
- **WHEN** the Socrates rule concludes from e1 = `(v:socrates rdf:type v:Human)`
- **THEN** the conclusion's proof is `{ rule: 0, premises: [e1] }`

#### Scenario: Fuse
- **WHEN** a rule `{ ?x a v:Cat. ?x a v:Dog } => false.` fires
- **THEN** the result reports a contradiction naming that rule

### Requirement: Reasoning surfaces
The JSON bridge SHALL accept a read operation `reason`, Node, Python and the wasm package SHALL expose it on their database objects, the MCP server SHALL provide a read tool `reason`, and the CLI SHALL provide `tm reason <file> (--rules <path> | --rule-set <iri>) [--as-of <t>] [--valid-at <instant> | --valid-during <from>..<to>] [--graph <iri>]… [--proof]`, all with the same options and result as `View::reason`.

#### Scenario: CLI reasoning
- **WHEN** `tm reason memory.db --rules socrates.n3` runs on a file holding `(v:socrates rdf:type v:Human)`
- **THEN** it prints the conclusion `(v:socrates rdf:type v:Mortal)` and exits 0

#### Scenario: Bridge reasoning
- **WHEN** the bridge receives `{"op": "reason", "rules": "{ ?x a <urn:tiramemsu:v:Human> } => { ?x a <urn:tiramemsu:v:Mortal> }."}`
- **THEN** it answers with the same conclusions and proofs as `View::reason`
