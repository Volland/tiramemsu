## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Extend the shared IR with the existing TimeRespecting after option and arrival binding. Specify and test grammar before implementation; any syntax is an opt-in Tiramemsu extension. Both frontends must use the existing native operator, graph scope, and snapshot. Stored hops move tau to max(tau,v_from) only when v_to is absent or greater than tau; virtual hops preserve tau. Reachability reports earliest arrival. Distinguish an explicitly bounded search domain from an unbounded expression stopped by a configured hop cap. State-budget exhaustion remains an error. Preserve ordinary query row shapes unless metadata/arrival is requested.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

- **IR.** `tm_ir::TemporalPath { after: Option<TermOrVar>, arrival: Option<Var> }` on `PathPattern.time_respecting`. `after` is an integer (epoch ms), an `xsd:date`/`xsd:dateTime` constant or a parameter; the executor binds parameters, resolves the instant, appends `timeRespecting[/<ms>]` to the `tm_path` view text and binds the `arrival` column (an integer, NULL for −∞). Both frontends therefore use the existing native operator in the pattern's own view, graph selection and statement snapshot. A time-respecting path is only called from its start: one bound only at its end is `Unsupported` (a journey read backwards is a different question).
- **SPARQL grammar.** `SERVICE <urn:tiramemsu:tm:timeRespecting>` or `…/timeRespecting/<t>` with `t` an integer of epoch ms, an `xsd:date`, an `xsd:dateTime` or `$name` (from the new `SparqlOptions::params`). `SERVICE` already carries time in this dialect, so no grammar change is needed. Every path of the group that reaches the lowering as a path (`*`, `+`, `?`, `^`, `|`) is one `REACH` region with the modifier; a plain sequence `a/b` is desugared to triple patterns by `spargebra` and is documented as not part of a journey. `?end tm:arrival ?t` (a magic pattern like `tm:textMatch`) binds the arrival of the one temporal path of the group ending at `?end`. A group with no path, `tm:arrival` outside a group or naming no/several paths, a malformed start and the modifier in `FROM`/`GRAPH` are `Parse` errors.
- **Cypher grammar.** `MATCH [mode] TIME RESPECTING [AFTER t] [ARRIVAL AS name]`, recognised and blanked by the span-preserving pre-pass like `REPEATABLE ELEMENTS`. `AFTER` takes the forms of `USE AS OF` / `VALID AT` (integer, parameter, `datetime()`, `date()`) plus a negative integer. Every variable-length and shortest-path relationship of the clause becomes time-respecting; `ARRIVAL AS` needs exactly one, binds a new variable (Integer or `null`) and is `null` for an unmatched `OPTIONAL MATCH`. Cypher has no graph selector, so its journeys are never graph-scoped.
- **Completeness.** `tm_ir::PathCompleteness` with `Exhaustive`, `StoppedAtBound { max_hops }` and `StoppedAtCap { max_hops }`. When the hop bound ends a search with entries left, one extra fetch round probes for a neighbour along a DFA transition; a bounded expression whose automaton has no move left is exhaustive, and the probe ignores time and trail identity (a cut means "longer paths may exist"). The cap is told from a query bound by `PathRequest.hop_cap` / `PathPattern.hop_cap` (set by Cypher for an unbounded upper limit), a `hopCap` view-text part on `tm_path`, or a NULL `max_hops` under `TRAIL`. Query results collect the verdicts of all `tm_path` calls through a thread-local scope (`tm_exec::path::report`), merged to the least complete. State exhaustion stays `PathLimitExceeded`.
- **Row shapes.** Without `tm:arrival` / `ARRIVAL AS` rows keep their columns. Rust results gain only side fields (`Solutions::path_completeness`, `CypherResult::path_completeness`); the bridge adds `pathCompleteness` or the `{rows, completeness}` form of `path` only when asked. No storage format change.
- **Not done.** MCP tools do not expose completeness (their query tool already reports provenance coverage); latest-departure and fastest journeys remain open.
