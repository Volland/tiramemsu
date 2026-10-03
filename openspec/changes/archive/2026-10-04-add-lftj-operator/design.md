## Context

This proposal follows `docs/project-review.md`. It is a future change and is not part of the resource-recovery patch.

## Decisions

Use the existing NativeKind::Lftj extension point with snapshot-local, sorted access paths. Initially support pure cyclic triple-pattern BGPs; leave OPTIONAL, UNION, aggregates, and unsupported mixed regions on SQL. Each access path carries its own temporal view and graph membership constraints. Preserve SPARQL set-of-triples versus Cypher bag-of-eids semantics, including parallel edges and provenance bindings. Do not route solely because the enabled flag is set: require an installed operator, supported shape, and a documented estimate policy. Start disabled by default. Add explain visibility and bound memory with shared query budgets.

## Compatibility and rollout

Use opt-in APIs or packages first. Preserve existing file invariants and ordinary query behavior. Any schema addition requires a new format migration; derived indexes may be rebuilt but graph history may not be rewritten. Keep unsupported behavior explicit in Rust and binding errors.

## Risks and validation

Verify normal operation, errors, cancellation or interruption where applicable, and reopen behavior. Use existing temporal/dialect differential fixtures for shared semantics. No performance claim is accepted without matching result counts and representative data.

## Open decisions

Exact public API names, package names, and any new frontend grammar must be finalized before implementation. The requirements in the delta spec define behavior; API spelling in this draft is descriptive.

## Resolution

- **Boundary.** The operator implements `NativeKind::Lftj` as the eponymous table function `tm_lftj(spec)` with columns `c0 … c31`. The planner writes the whole region plan (patterns in join order, constants, per-pattern views, canonical flag, isomorphism group, output variables) into the text argument, so the operator holds no state and composes with the surrounding SQL like `tm_path`. The call sits in an unflattened derived table (`LIMIT -1`) so an inner-loop position materialises it once.
- **Access paths.** Each pattern is scanned through the calling statement's connection (same snapshot, or the speculative state in `with`) with SQL produced by the same pattern generator as the SQL route: constants, its own view predicates and, under set semantics, the canonical-eid predicate, ordered by the join order. `GRAPH` selectors are already membership patterns, so graph constraints are access paths with their own views.
- **Semantics.** Every pattern carries an eid variable (hidden when unbound), so bag multiplicities and parallel edges survive; SPARQL's canonical-eid predicate gives set semantics; Cypher isomorphism is checked per binding with the SQL route's rule, and grouped eids are output columns for patterns outside the region (`IsoPat` now holds the eid SQL instead of an alias).
- **Routing.** `route_bgp` then `lftj_route`: enabled, installed (the facade installs the operator only when enabled), only stored-triple inputs and at most 32 output variables, and the estimate policy: some pattern matches at least `min_rows_estimate` statements in its view, counted with a capped `count(*)` in the planning snapshot (default 100 000; 0 skips it). Each failure is an explain note; cycles closed only through `OPTIONAL` or among non-pattern inputs report `LftjUnsupportedShape`.
- **Budgets.** The operator checks the operation budget at start and polls it while scanning and joining; a cancelled or timed-out call fails the statement with the typed error and no rows. Row and byte budgets apply to the rows the statement returns. The output is materialised before SQLite reads it, which bounds nothing by itself: memory grows with the region's output (documented in `lat.md/query.md`).
- **Evidence.** Differential tests compare both routes on one file (set, bag, isomorphism, mixed views, graphs, provenance, a property test over random histories). The triangle harness checks equal counts before timing; at the small scale the native route was 1.3× (uniform) to 12× (hub-and-spoke) faster. The full scale is not yet measured.
