# Store and enforce ELPI programs

Source: decisions E5, E6 and E7 of `lat.md/elpi.md` (interview of 2026-10-31). Change 2 of four; depends on `add-elpi-engine`.

## Why

A validator that is not stored cannot be enforced, versioned or shared. Programs as data put ELPI on the same footing as Datalog programs (DL16). Enforcement turns `tm.violation-delta` into a guard on every commit.

## What Changes

- **New crate `tm-elpi-store`:** persistence of ELPI programs over `tm-elpi`, `tm-core` and `tm-store`. It writes only through the transaction handle.
- **Program nodes:** `(program elpi:source "<text>")`, edited by supersede; equal text writes nothing. `RuleLanguageConflict` applies in both directions with `n3:source` and `dl:source`.
- **Includes:** `accumulate <program-iri>.` resolves from the same snapshot to the included program's live source. A cycle fails with `ProgramCycle` and an unknown program with `UnknownProgram`. `program_info` lists the closure.
- **Enforcement:** `(program elpi:enforced true)`. Turning it on, or changing an enforced program's text, runs the full `tm.violation` check, and any `error` violation fails the transaction. Every other commit runs `tm.violation-delta` seeded by the transaction's added and retracted statements, under the constraint budget: lean-elpi fuel plus a row cap, failing closed with `ConstraintBudgetExceeded`. A violation the view as of the previous transaction already held is not counted.
- **Facade:** `TxElpi::store_elpi_program`, `TxElpi::enforce_elpi`, `View::elpi_program_info`, `ElpiProgram::Stored`, and the commit hook beside Datalog's.
- **CLI:** `tm elpi program put|show|enforce|unenforce`.

## Capabilities

### New Capabilities

- `elpi-programs`: stored programs, includes and program information.
- `elpi-constraints`: enforcement at commit.

## Impact

- New `crates/tm-elpi-store`; the facade's `src/elpi.rs` and `src/db.rs` commit hook; `tm-core` vocabulary `ELPI_SOURCE`, `ELPI_ENFORCED` and the error `ProgramCycle`; `TxN3::store_rules` and `TxDatalog::store_program` also refuse a node with `elpi:source`.
- lat.md: `lat.md/elpi.md` (E5–E7 implemented).
- Binding constraints: [[constraints#Single Writer]], [[constraints#Never Forget]].
