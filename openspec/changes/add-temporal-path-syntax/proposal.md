# Temporal path syntax and completeness reporting

## Status

Implemented. Final API: the SPARQL scope `SERVICE <urn:tiramemsu:tm:timeRespecting[/<t>]>` with `?end tm:arrival ?t` and `SparqlOptions::params` for `$name` starts; the Cypher modifier `MATCH TIME RESPECTING [AFTER t] [ARRIVAL AS name]`; the IR option `tm_ir::TemporalPath` (`PathPattern.time_respecting`) and `PathPattern.hop_cap`; `tm_ir::PathCompleteness` (`Exhaustive`, `StoppedAtBound`, `StoppedAtCap`) reported by `View::path_report`, `Solutions::path_completeness` and `CypherResult::path_completeness`, with `PathArgs::capped`; the bridge options `params`, `pathCompleteness`, `capped` and `completeness`, and their Node and Python wrappers.

## Why

The native engine already supports causal journeys, but SPARQL and Cypher cannot express them. Bounded path results also need explicit completeness information.

## What Changes

Define frontend temporal path modifiers and arrival bindings, and expose path completeness metadata without changing existing path syntax.

## Capabilities

### New Capabilities

- `temporal-language-paths`: temporal path syntax and completeness reporting.

### Modified Capabilities

None in this draft. Implementation must add delta modifications if an existing public requirement must change.

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No second temporal search engine.
- No change to ordinary SPARQL reachability or Cypher trail semantics.
