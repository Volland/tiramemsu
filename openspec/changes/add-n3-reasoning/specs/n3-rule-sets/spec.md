## ADDED Requirements

### Requirement: Store a rule set
`Tx::store_rules(rule_set, n3)` SHALL parse `n3` and, if it parses, assert `R = (rule_set n3:source "<n3>")`, where `n3:` is `urn:tiramemsu:n3:`. When a live `n3:source` statement for `rule_set` exists with different text, it SHALL be superseded by the new text; with equal text the call SHALL return the existing statement. Text that does not parse SHALL fail with `N3Parse` and leave no trace. `Rules::Stored(rule_set)` SHALL read the live `n3:source` of `rule_set` in the reasoning view and fail with `UnknownRuleSet` when there is none.

#### Scenario: Store and reason
- **WHEN** `store_rules(v:mortality, "{ ?x a v:Human } => { ?x a v:Mortal }.")` commits and `reason(Rules::Stored(v:mortality))` runs
- **THEN** it concludes as if the text had been passed directly, and the proofs name `v:mortality`

#### Scenario: Re-storing the same text
- **WHEN** the same text is stored twice
- **THEN** the second call returns the first statement's eid and supersedes nothing

#### Scenario: Unknown rule set
- **WHEN** `reason(Rules::Stored(v:nothing))` runs
- **THEN** it fails with `UnknownRuleSet`

### Requirement: Rule sets have transaction time
Editing a rule set SHALL be a supersede, so `as_of(t)` reasons with the rule set believed at `t` and the history keeps every version.

#### Scenario: Rules as they were
- **WHEN** rule set version A is stored in transaction 2, version B in transaction 5, and `reason(Rules::Stored(rs))` runs on `as_of(Tx(3))`
- **THEN** it runs version A

### Requirement: Monotonicity check
Parsing a rule set SHALL classify it: `Monotonic`, `NonMonotonic { rule, builtin }` when a rule body uses `log:notIncludes`, `log:collectAllIn` or `log:forAllIn`, or `ClockDependent { rule, builtin }` when it uses `time:localTime` or `tm:now`. `View::rule_set_info(rule_set)` SHALL report the classification, the rule count and the predicates its bodies use.

#### Scenario: Negation is detected
- **WHEN** a rule set contains `{ ?x a v:Employee. ?SCOPE log:notIncludes { ?x v:manager ?m } } => { ?x a v:Unmanaged }`
- **THEN** it is classified `NonMonotonic` naming rule 0 and `log:notIncludes`

### Requirement: Rule-set surfaces
The JSON bridge SHALL accept the transaction operation `storeRules`, Node and Python SHALL expose it on their transaction builders, the MCP server SHALL provide a write tool `store_rules` absent in read-only mode, and the CLI SHALL provide `tm rules put <file> <iri> <path>` and `tm rules show <file> <iri>`.

#### Scenario: MCP read-only mode hides rule writes
- **WHEN** the MCP server runs with `--read-only`
- **THEN** `store_rules` and `materialize` are not listed and `reason` is
