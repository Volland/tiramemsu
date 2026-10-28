# Datalog in Tiramemsu: a guide

Datalog is how Tiramemsu answers "what follows from what we know", "what must never be true" and "what should be remembered because it follows", over a memory with two clocks. This guide is the practical companion to the [long read](../site/articles/datalog-temporal-reasoning.html): the language, the time operators, constraints, materialization and where each call lives. Design decisions are in `lat.md/datalog.md` (DL1 to DL26).

Datalog is the opt-in facade feature `datalog` (crate `tm-datalog`, published to crates.io). It does not need the query engine, so `--no-default-features --features datalog` is enough. In the examples `v:` is `urn:tiramemsu:v:`, one of the prefixes every program may use without declaring: `rdf`, `rdfs`, `xsd`, `owl`, `dl`, `sys`, `tm` and `v`.

## Five minutes

Store two statements and a program, then ask a question:

```rust
let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
db.transact(TxOptions::default(), |tx| {
    tx.assert(v("alice"), v("manager"), v("bob"), Valid::ALWAYS)?;
    tx.assert(v("bob"), v("manager"), v("carol"), Valid::ALWAYS)?;
    Ok(())
})?;
let program = DatalogProgram::Text(
    "reports(X, Y) :- v:manager(X, Y).\n\
     reports(X, Z) :- reports(X, Y), v:manager(Y, Z).".into(),
);
let r = db.now().datalog(&program, Some("reports(v:alice, X)"), &DatalogOptions::default())?;
// two rows: bob and carol, each with the valid time over which it holds
```

From the command line the same program in a file `reporting.dl` is `tm datalog mem.db reporting.dl "reports(v:alice, X)"`; the bridge op is `datalog`, the MCP tool `datalog_query`.

## The language

A program is text. `%` starts a comment. It holds:

| Item | Form |
|---|---|
| Prefix | `@prefix ex: <http://example.org/> .` |
| Rule | `head :- body.` (a fact is a head with no body) |
| Goal | `?- body.` |
| Constraint | `constraint name warn :- body.` or `constraint name error :- body.` |

Variables start with an upper-case letter or `_`; a bare `_` is anonymous. Constants are prefixed names, `<iri>`, strings, numbers, booleans and typed literals. Durations are numbers with a unit: `ms`, `s`, `m`, `h`, `d`, `w` (`30d`).

**Atoms.** `ex:p(S, O)` reads the live statements whose predicate is `ex:p`, under set semantics on `(S, O)`. `ex:p(S, O) @ E` also binds the statement's id, one row per statement, which is how you reach layers: `v:worksAt(X, C) @ E, v:confidence(E, K), K > 0.8`. A predicate IRI that a rule head also defines is a derived relation that holds the stored statements as well, so stored and derived facts combine. Plain lower-case names (`reports`) are derived relations of any arity.

**Reserved atoms** reach the rest of a statement:

| Atom | Meaning |
|---|---|
| `triple(S, P, O)` | any predicate (a variable predicate turns pruning off); a fact, valid over the statement's time |
| `stmt(E, S, P, O)` | the statement's record; holds at every instant |
| `valid(E, From, To)` | the statement's valid interval (`dl:inf` for an open end) |
| `added(E, T)`, `tx_instant(T, Ms)` | the transaction that added it, and its instant |
| `in_graph(E, G)` | named-graph membership |

**Builtins.** Comparisons `= != < <= > >=` (the wrong kind of value makes the comparison false, never an error); arithmetic `+ - * / %` on numbers, and on instants and durations (`D + 30d`); `concat`, `strlen`, `lower`, `upper`, `contains`, `starts_with`; `now()`, the view's pinned instant.

**Negation and aggregates** are stratified. `not v:manager(X, _)` needs `X` bound by a positive atom. Aggregates are `N = count : { body }` and `S = sum V : { body }` (also `min`, `max`). A program that recurses through negation or an aggregate is refused with the cycle named (`DatalogUnstratifiable`).

**Invention.** `exists M: v:assignedManager(X, M) :- rdf:type(X, v:Employee), not v:manager(X, _).` invents a node per `X`. The same firing reuses the same node. Recursion through invention needs `limits.max_new_values` (`DatalogUnboundedInvention`).

## Time

A derived fact holds at an instant exactly when it follows from the statements valid at that instant. Facts that were never valid together never combine. Results carry their valid time as a set of half-open intervals of epoch milliseconds.

```
v:worksIn(P, L) :- v:worksAt(P, C), v:locatedIn(C, L).
```

With `worksAt` valid 2020 to 2025 and `locatedIn` valid from 2024, `worksIn` holds 2024 to 2025. `time: atemporal` ignores valid time and makes every result unbounded. A `valid_at(t)` view gives intervals `[t, t+1)`. Transaction time is the view's: an `as_of` view runs the program that was stored then over the facts believed then, and `now()` is that view's instant.

### DatalogMTL operators

Body operators take a window `[a, b]` of durations (`b` may be `inf`):

| Operator | A holds at `t` when... |
|---|---|
| `past[a,b] A` | A held at some instant in `[t−b, t−a]` |
| `always_past[a,b] A` | A held throughout `[t−b, t−a]` |
| `future[a,b] A`, `always_future[a,b] A` | the same, looking forward |
| `A since[a,b] B` | B held at some `s` in the window and A at every instant after it |
| `A until[a,b] B` | B holds at some `s` in the window and A at every instant before it |

```
alert(X)    :- past[0d, 3d] v:fever(X, true).
tenured(X)  :- always_past[0d, 30d] v:employed(X, true).
pending(S)  :- v:unacked(S, true) since[0, inf] v:alarm(S, true).
always_future[0d, 7d] blocked(R) :- v:booked(R, true).
```

Heads may only use the `always_*` operators; `past` or `future` in a head is not Horn and fails with `DatalogNotHorn`. **Recursion through an operator needs a window**: a program like `contagious(Y) :- past[0d, 3d] contagious(X), v:contact(X, Y).` can push intervals forward forever, so it fails with `DatalogUnboundedRecursion` unless you pass `window` (a clip for every iteration).

### Allen relations

`before`, `meets`, `overlaps`, `starts`, `during`, `finishes`, `equals` and the converses `finishedBy`, `contains`, `startedBy`, `overlappedBy`, `metBy`, `after`, plus `intersects`, are builtins over statement ids or intervals; `interval(E, I)`, `start(I, T)`, `end(I, T)` and `duration(I, D)` take intervals apart. Use `stmt` records when the two statements were never valid together:

```
stmt(E1, v:a, v:job, v:x), stmt(E2, v:a, v:job, v:y), meets(E1, E2)
```

The same goal through facts (`v:job(v:a, v:x) @ E1, ...`) has no row, because the two jobs are never valid at one instant.

## Constraints

```
constraint one_ceo error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y.
constraint owned   warn  :- rdf:type(P, v:Project), not v:owner(P, _).
```

A constraint is a body that must have no match. `View::check` (bridge `check`, MCP `check_constraints`, CLI `tm check`) returns each violation with its bindings, the statement ids it rests on and the intervals on which it holds, on any view. Because it is temporal, successive CEOs are fine and overlapping ones are a violation on exactly the overlap.

**Enforcement.** `tx.enforce(program, true)` (bridge `enforce`, MCP `enforce_program`, CLI `tm program enforce`) writes `(program dl:enforced true)`. From then on every commit checks the program's `error` constraints from the statements it added and retracted only, under `OpenOptions::constraint_budget` (default 50 ms and 100 000 seed rows). A violation rolls the commit back with `ConstraintViolated`; over budget it fails with `ConstraintBudgetExceeded`. `warn` violations are listed in the transaction report; a dry run lists both; a `with` speculation fails like a commit. Enforcing a program that live data already violates is refused, listing the violations. An `error` constraint must be enforceable: stored atoms, non-recursive derived relations (inlined), and metric operators with finite windows. Aggregates, `since`/`until` and recursive relations make it check-only (`ConstraintNotEnforceable`).

## Stored programs

`tx.store_program(node, text)` keeps the text as the one statement `(node dl:source "...")`. The text is parsed and analysed first, so an invalid program is never stored. Editing is a supersede, so an `as_of` view runs the program believed then. A node holds rules of one language: storing Datalog where N3 rules live (or the reverse) fails with `RuleLanguageConflict`. `View::program_info` reports the rules, strata, constraints, the predicates read and whether the program recurses through an operator or through invention. Run a stored program with `DatalogProgram::Stored(node)`.

## Materialization

`Db::materialize_program(node, &opts)` (bridge `materializeProgram`, MCP `materialize_program`, CLI `tm materialize-program`) evaluates the program on a snapshot and writes the conclusions of its IRI relations as **derived beliefs**: ordinary statements whose support is their premises plus the program statement. Retract a premise or edit the program and the conclusions go with it, in the same transaction. One statement is written per interval on which one derivation holds; a later run over a longer interval widens it; invented nodes are reused. A run never reads its own earlier conclusions, so running it again writes nothing but the transaction label `(tx dl:materialized program)`; `dry_run` reports without writing.

Strata that use negation or aggregates are **reconciled**: a conclusion the run no longer derives is retracted, and the transaction is labelled `dl:reconciled` with a count. Such a stratum can go stale between runs, because a later write does not retract anything, so the report lists their predicates in `needs_rerun_after_writes`.

## Goals inside SPARQL and Cypher

```sparql
SELECT ?X ?mail WHERE {
  SERVICE <urn:tiramemsu:dl:query> { [] dl:program v:reporting ; dl:goal "reports(v:alice, X)" }
  ?X v:email ?mail }
```

```cypher
CALL tiramemsu.datalog.query('v:reporting', 'reports(v:alice, X)') YIELD X, validFrom, validTo
```

The program (a stored node with `dl:program`, or inline with `dl:text`) and the goal must be constants or parameters. The call runs once per query, in the query's snapshot and under its deadline. A goal value the store has no id for (a computed string, an invented node) fails the call with `Unsupported`; ask it through `View::datalog`.

## Running it

`DatalogOptions` (the bridge takes the same names in camelCase): `scope` (the whole view, named graphs, or seeds with hops), `window`, `time` (`Pointwise` or `Atemporal`), `limits` (`max_iterations`, `max_facts`, `max_fragments`, `max_new_values`), `demand` (magic sets for goals with constants, on by default), `proof` (one derivation per row), `access` (`Auto`, `Probe`, `Load`), `parallel_threshold` and `require_complete` (turn a reached limit into `DatalogIncomplete`). A run that hits a limit returns `Incomplete` with the limit named, and every row it returns is still a true consequence. `View::explain_datalog` shows the strata, join order and each predicate's access.

## What to know before relying on it

- **Not built:** incremental materialization (every run evaluates the whole program), and a worst-case-optimal join for cyclic rule bodies.
- **Speed:** goal queries with constants run in milliseconds; full closures are 3 to 4 times slower than Soufflé's interpreter and 6 to 7 times faster than N3 on a hub graph. Figures and caveats are in `bench/datalog/README.md`.
- **Errors:** `DatalogParse`, `DatalogUnsafe`, `DatalogUnstratifiable`, `DatalogReserved`, `DatalogNotHorn`, `DatalogUnboundedRecursion`, `DatalogUnboundedInvention`, `DatalogTemporalOperatorInAtemporal`, `DatalogDuplicateConstraint`, `DatalogIncomplete`, `DatalogCallShape`, `UnknownProgram`, `RuleLanguageConflict`, `ConstraintViolated`, `ConstraintNotEnforceable`, `ConstraintBudgetExceeded`.
- **N3 or Datalog:** both are in the same store and share the support cascade. Reach for N3 for quoted formulas, N3 builtins and proof text; for Datalog when you need speed on recursion, temporal operators, enforced constraints or negation that materializes.
