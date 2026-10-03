# Optional query frontend dependencies

## Status

Implemented. Final API: `tiramemsu` cargo features `exec`, `sparql` and `cypher` (each front end implies `exec`), `default = ["sparql", "cypher"]`; `default-features = false` is the core tier with no query engine and no parser. RDF terms and the N-Triples writer live in `tm_core::rdf`. Saved answers need both front ends. The JSON bridge, `tiramemsu-mcp` and the bindings keep the full facade. `scripts/feature-matrix.sh` plus the CI `features` job verify core, exec, sparql, cypher, default and the bindings.

## Why

The facade always links both parsers. Core-only consumers should be able to omit them without confusing the existing test-only sparql feature.

## What Changes

Introduce genuine optional dependencies and feature-gated facade exports for SPARQL, Cypher, and the shared query engine.

## Capabilities

### New Capabilities

- `optional-query-frontends`: optional query frontend dependencies.

### Modified Capabilities

None. The default build keeps every existing requirement; the only migration is for dependants that already used `default-features = false` (documented in the facade README).

## Impact

Affects the facade, applicable executor/host or frontend seams, tests, and design documentation. Preserve statement identity, never-forget invariants, per-view visibility, and current default behavior unless an explicitly reviewed migration changes it. See `design.md` for scope and dependencies.

## Non-goals

- No WASM host in this change.
- No new language grammar or storage format.
