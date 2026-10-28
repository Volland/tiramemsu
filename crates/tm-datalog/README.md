# tm-datalog

Datalog for [tiramemsu](https://github.com/Volland/tiramemsu): a semi-naive engine with
DatalogMTL over valid time, magic-set goals and integrity constraints, reading a store
through the `ReadStore` trait of `tm-core`. Most programs reach it through the
`tiramemsu` facade with its cargo feature `datalog` (`View::datalog`,
`View::datalog_reason`, `View::check`, `View::program_info`, `View::explain_datalog`,
`TxDatalog::store_program`).

```text
@prefix ex: <http://example.org/> .
reports(X, Y) :- ex:manager(X, Y).
reports(X, Z) :- reports(X, Y), ex:manager(Y, Z).
alert(X) :- past[0d, 3d] ex:fever(X, true).
headcount(C, N) :- rdf:type(C, ex:Company), N = count : { ex:employs(C, _) }.
constraint one_ceo error :- ex:role(X, ex:CEO), ex:role(Y, ex:CEO), X != Y.
?- reports(ex:alice, X).
```

- **Atoms.** `p(S, O)` reads the statements of predicate `p` under set semantics,
  `p(S, O) @ E` also binds the statement eid. `triple(S, P, O)` reads any predicate;
  `stmt(E, S, P, O)`, `valid(E, From, To)`, `added(E, T)` and `tx_instant(T, Ms)` are
  statement records that hold at every instant; `in_graph(E, G)` reads graph
  membership. A predicate IRI that a head defines also holds its stored statements.
- **Time.** A fact holds at an instant exactly when it follows from the statements
  valid then. Tuples carry coalesced interval sets of epoch milliseconds, so one pass
  answers for the whole timeline. Bodies may use `past`, `always_past`, `future`,
  `always_future`, `since` and `until` over `[a, b]` windows, heads `always_past` and
  `always_future`, and the 13 Allen relations. Recursion through an operator needs a
  `window`.
- **Negation and aggregates** (`not`, `count`, `sum`, `min`, `max`) are stratified and
  evaluated at every instant: negation is the complement of a binding's set, an
  aggregate is a sweep over its group's endpoints.
- **Goals with constants** are rewritten for demand (magic sets); relations whose
  rules use negation, aggregates, operators or invention are evaluated in full.
- **Invention.** `exists Y:` heads invent nodes keyed by the rule and its frontier;
  recursion through invention needs `limits.max_new_values`.
- **Access.** A stored predicate is probed through the store's permutations and
  loaded into memory once it has been probed more than an eighth of its size.
- **No unsafe code, no git dependency**: the crate is publishable; join steps over
  large deltas run as parallel morsels (feature `parallel`, compiled out on wasm32).

Decisions DL1–DL26 are in the repository's `lat.md/datalog.md`; the specification is
OpenSpec change `add-datalog-engine`.
