# Add the ELPI engine

Source: decisions E1–E3, E6, E8, E9 and E11 of `lat.md/elpi.md` (interview of 2026-10-31). Change 1 of four; depends on lean-elpi 0.4 (host callback builtins and structured results in its C ABI).

## Why

N3 and Datalog cover rules over triples, but neither has higher-order clauses, hypothetical reasoning (`=>`, `pi`) or constraint handling rules. λProlog has all three, and lean-elpi runs the ELPI dialect of it behind a C interface. Validators that ask "would this still hold if…" and rules over rules are natural there and awkward in both existing languages.

## What Changes

- **New crate `tm-elpi`:** the engine over `leanelpi` 0.4 and `tm-core`/`tm-store`. It reads a view through host builtins (`tm.triple`, `tm.added`, `tm.retracted`, `tm.valid`), maps terms both ways, and never writes. Everything that touches FFI stays inside `leanelpi`, so the crate forbids unsafe code.
- **Runs:** goal queries, the full `tm.violation` check, the delta check `tm.violation-delta`, and `tm.conclude` runs that return conclusions with the support each one states. Every eid a program lists in its support is checked against the eids the run's callbacks returned.
- **Time:** `query` and `check` read at one valid instant. `conclude` gives each conclusion the intersection of its support's valid intervals, which is the pointwise result for programs the monotonicity scan accepts. Other programs are swept segment by segment.
- **Facade feature `elpi`, off by default:** `View::elpi` (goal rows), `View::elpi_check` and the `ElpiProgram` source type. The library comes from `LEANELPI_DIST`.
- **Surfaces:** CLI `tm elpi query|check`.

## Capabilities

### New Capabilities

- `elpi-engine`: λProlog programs over a view: builtins, terms, queries, checks, conclusions, support checks, time.

## Impact

- New `crates/tm-elpi`. Facade `crates/tiramemsu` (`src/elpi.rs`, feature `elpi`), `tm-core` (errors `ElpiProgram`, `ElpiRun`, `ElpiOutOfFuel`). CLI `crates/tiramemsu-cli`.
- The workspace depends on `leanelpi = "0.4"`. Until 0.4 is on crates.io a `[patch.crates-io]` entry points at a local lean-elpi checkout.
- lat.md: `lat.md/elpi.md` (E1–E3, E6, E8, E9 implemented), `lat.md/architecture.md#Crates`.
- Binding constraints: [[constraints#No Unsafe Code]], [[constraints#Publishable Crates]], [[reasoning#Optional Crate]].

## Non-goals

- Stored programs, enforcement and materialization (changes 2 and 3), SPARQL and Cypher calls (change 4).
- Node, Python and wasm packages with ELPI (E10).
