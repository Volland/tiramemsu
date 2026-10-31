# tm-elpi

ELPI for [tiramemsu](https://github.com/Volland/tiramemsu): λProlog programs, run by
[lean-elpi](https://github.com/Volland/lean-elpi), that read a view through host
builtins. The crate never writes; stored programs, enforcement and materialization
live in `tm-elpi-store`. Most programs reach it through the `tiramemsu` facade with
its cargo feature `elpi` (`View::elpi`, `View::elpi_check`).

```text
reports X Y :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _.
reports X Z :- tm.triple X (iri "urn:tiramemsu:v:manager") Y _, reports Y Z.

tm.conclude (triple X (iri "urn:ex:grandparent") Z) [E1, E2] :-
  tm.triple X (iri "urn:ex:parent") Y E1, tm.triple Y (iri "urn:ex:parent") Z E2.

tm.violation error "one_ceo" "two CEOs" [E1, E2] :-
  tm.triple X (iri "urn:ex:role") (iri "urn:ex:CEO") E1,
  tm.triple Y (iri "urn:ex:role") (iri "urn:ex:CEO") E2, not (X = Y).
```

- **Builtins.** `tm.triple S P O E` reads the view's statements, and bound arguments
  become index lookups. `tm.added` and `tm.retracted` read a transaction's changes
  during a commit-time check, and `tm.valid E From To` reads a statement's interval.
- **Terms.** `(iri S)`, `(str S)`, `(num N)`, `(dbl F)`, `(bool S)`,
  `(lang Lex Tag)`, `(lit Lex Datatype)`, `(node N)`, `(bnode N)`, `(tx N)` and
  `(stmtv (eid N))`. Intervals are bounded by `(at Ms)` or `inf`.
- **Support.** `tm.conclude Fact Support` lists the eids a conclusion rests on. The
  run refuses an eid its builtins never returned.
- **Time.** Queries read at one valid instant. A conclusion holds over the
  intersection of its support's intervals. Programs that use negation, cut or
  `findall` are evaluated per valid-time segment.

The cargo feature `native` links `libleanelpi`, found through `LEANELPI_DIST`.
Without it every run fails with `Unsupported`.
