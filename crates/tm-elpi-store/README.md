# tm-elpi-store

Persistence of ELPI programs for [tiramemsu](https://github.com/Volland/tiramemsu),
over the engine crate `tm-elpi`. A program is a node with one live
`(program elpi:source "<text>")` statement. Editing it is a supersede, so a view as of
an earlier transaction runs the program believed then.

- **Includes.** A line `accumulate <iri>.` includes the live source of another stored
  program from the same view. Cycles fail with `ProgramCycle`.
- **Enforcement.** `(program elpi:enforced true)` turns the program's constraints on.
  The commit hook runs `tm.violation-delta`, seeded by the transaction's added and
  retracted statements. When enforcement is turned on, or the text changes, it runs
  the full `tm.violation` check instead.
- **Materialization plans.** `plan` runs `tm.conclude` with the program's own
  conclusions hidden. It returns items whose support is the eids they state, plus the
  program's source and its included sources. The facade writes the items through its
  shared materialization commit.

The crate writes only through the transaction handle of `tm-core`. Most programs
reach it through the `tiramemsu` facade with its cargo feature `elpi`
(`TxElpi::store_elpi_program`, `TxElpi::enforce_elpi`, `Db::materialize_elpi`).
