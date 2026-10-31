# λProlog in a bitemporal memory

*How Tiramemsu gained a third reasoning language: ELPI, run by an implementation written in Lean 4, reading the store through callbacks, stating its own support, and checking writes at commit.*

---

Tiramemsu already had two ways to reason over memory. N3 rules run through eyeron, and a Datalog engine of its own handles recursion, constraints and valid time in one pass. Both are good at rules over triples, and both are flat. Neither lets a rule quantify over other rules, assume a fact for the length of one proof, or say "for any fresh x". Those are exactly the questions an agent's validator asks. Would this plan still hold if Bob left the team? Is every role assignment justified by some policy, whatever the policy is? Does a schema accept this record under every reading of its optional fields?

λProlog has these constructs built in. `=>` adds a clause for the length of one goal, `pi x\` introduces a fresh name, and clauses can take other predicates as arguments. ELPI is the dialect of λProlog that Coq and Lean tooling use, and lean-elpi is an implementation of it in Lean 4 with a C interface. This article is about wiring it into a store that has two clocks and never forgets.

## Callbacks, not copies

The first design question was how a program sees the store. The easy answer is to load the statements into the program as facts, `triple S P O E.`, and solve. That works with lean-elpi as it was, and it is wrong for two reasons.

The first reason is cost. The time spent loading grows with the scope on every run, even when the question touches three statements.

The second reason decided it: commit-time checks. A constraint enforced at commit must cost what the write costs, not what the memory costs, because it runs inside the single writer. Loading the memory for every transaction is the opposite of that.

So lean-elpi grew a host callback interface in version 0.4. A program declares a predicate, the host passes a function pointer at solve time, and every call of that predicate goes to tiramemsu. Tiramemsu declares four of them:

- `tm.triple S P O E` reads statements;
- `tm.valid E From To` reads a statement's interval;
- `tm.added` and `tm.retracted` read what the current transaction changed.

A bound argument becomes an index lookup. `tm.triple (iri "…alice") P O E` scans alice's statements and nothing else. A value that is not in the dictionary short-circuits to no rows without touching the store.

The same release made answers structured. lean-elpi used to return each binding as printed ELPI text. Parsing printed λProlog back into values is fragile: escapes, operators, and names introduced by `pi`. The 0.4 interface returns a JSON term tree instead, with constants, applications and native integers, floats and strings. The changes are in lean-elpi's own repository and stay within its C shim and Rust binding, so the tiramemsu crates still forbid unsafe code.

## Terms that carry their type

ELPI is typed, which rules out the obvious mapping of an integer to an ELPI `int` and an IRI to an ELPI string. `tm.triple`'s object argument has to have one type, so every tiramemsu value is a `term` built by one of a dozen constructors in a prelude: `(iri "…")`, `(str "…")`, `(num 41)`, `(dbl 1.5)`, `(lang "Hallo" "de")`, `(lit Lex Datatype)` and so on. Matching `(num N)` unwraps the native integer, so `M is N + 1` just works. This was a refinement found while building: the plan said "native values", and the type checker said "under a constructor".

## Support the program states

Tiramemsu's rule for a derived belief is that it carries one `sys:derivedFrom` link per premise, so retracting a premise retracts the belief. Datalog finds premises by tracking derivations. N3 reads them from eyeron's proofs. ELPI has backtracking, cut, hypothetical clauses and higher-order calls, so "the facts this answer used" is not a by-product of the search.

One option was to record every statement the callbacks returned. That over-approximates: a branch that failed still counts, and the conclusion would fall when an unrelated fact changed. Another was proof terms from lean-elpi's LF layer. Those are precise, but they are a large interface and slow.

The design we chose is that the program says what it used. Conclusions are clauses of `tm.conclude (triple S P O) [E1, E2]`, where the list holds the statement ids the clause read from `tm.triple`. λProlog programmers thread witnesses like this anyway. The host trusts none of it blindly. Every listed statement must be one the run's callbacks actually returned, so a program cannot claim support it never read, and an empty list is refused. The materialized conclusion is then supported by those statements, by the program's source statement, and by the source of every program it includes with `accumulate`. Editing a shared library therefore retracts the conclusions of every program built on it, which the tests check.

## Valid time without a sweep, mostly

N3 handles valid time by sweeping: it reasons once per segment between interval bounds. With support explicit, most ELPI programs do not need that. For a program without negation, a conclusion holds exactly when all of its premises hold, so its interval is the intersection of theirs. Premises valid over `[2020, 2025)` and `[2022, ∞)` give `[2022, 2025)`. Premises that were never true together give nothing, and the conclusion is dropped.

A program that uses `not`, cut, `findall` or `tm.retracted` is different, because absence matters there, and absence has no interval. A textual scan labels such programs non-monotonic. They are evaluated once per segment, cut at the bounds of every statement the first run read. Materializing them always reconciles: earlier conclusions the new run does not derive are retracted. A "person without a manager" conclusion goes away when the manager is added and the program runs again.

## Constraints that cost what the write costs

Datalog's commit-time checks unfold a constraint body automatically, one variant per atom, each starting from the transaction's changes. ELPI programs cannot be unfolded that way: they are Turing-complete and higher-order. So a program states two forms. `tm.violation` is the full check, run when enforcement is turned on or the text changes. `tm.violation-delta` starts from `tm.added` or `tm.retracted`, and only it runs on ordinary commits. A delta form that misses a case weakens enforcement but never corrupts the store, and the full check still finds what it missed.

To find out whether hand-written delta forms can be trusted, a property test builds random histories of role, project and owner writes. Each transaction is offered to an enforced database and to an unenforced shadow. The test checks that the enforced database refuses the transaction exactly when the full check on the shadow finds a violation the transaction introduced. 300 random cases pass.

The test also failed once, instructively. Its first version compared the two databases by statement id, and the ids drifted. A speculative `with` transaction rolls back its writes but not the eid counter, so after one speculation the shadow's next statement number was one higher. That is not a bug in the store, since ids only need to be unique, but it was a bug in the test, which now compares statements by content.

## Three things the tests found

**A loop that took 27 seconds.** The first commit-time budget turned the row cap into fuel at a thousand resolution steps per row: 10⁸ steps. lean-elpi runs a few million steps a second, so a constraint that looped held the writer for 27 seconds before failing. The budget is now derived from the commit deadline instead: 4000 steps per millisecond, so a looping check fails close to the 50 ms default, and the whole test suite went from 54 seconds to one.

**A library nobody could find.** Every test aborted with SIGABRT under `cargo test` and passed when run by hand. lean-elpi's build script set an rpath with `cargo:rustc-link-arg`, which applies only to targets of the crate that emits it, so dependent test binaries had none. Cargo does put library search paths on the loader path of `cargo test`, but only paths inside the target directory. The fix was in lean-elpi: its build script now copies the library into its own `OUT_DIR`.

**Clauses that could not see their builtins.** Declaring the host predicates only through the C interface made every program that called them fail to typecheck: the program was compiled before the declarations existed. The prelude now declares them as ordinary predicates as well, and the interface accepts a host declaration for a predicate the program declared but defined no clauses for.

## One path for goal calls

A Datalog goal can already sit inside a SPARQL or Cypher query as a table. Making ELPI goals do the same took a language tag, not a second mechanism. The IR leaf and the plan node carry `GoalLanguage::Datalog` or `GoalLanguage::Elpi`, and the engine holds one provider per language. The SQLite table function and the redb operator both pick the provider by that tag. `SERVICE <urn:tiramemsu:elpi:query>` and `CALL tiramemsu.elpi.query(program, goal)` were then a few dozen lines in each front end.

ELPI goals read one valid instant, so their rows carry no interval. A query row holds only stored terms, so a value the program computed is refused with `Unsupported` rather than invented.

## What it costs, and what is missing

lean-elpi is about 2.4 times slower than ELPI 1.18 by geometric mean on ELPI's own benchmark programs, and tiramemsu has no ELPI benchmark of its own yet. So no speed claim is made here. Single test scenarios run in about a tenth of a second, and an ELPI closure over a pseudo-random graph returns the same pairs as Datalog's, which is the oracle the tests use.

The native library carries the Lean runtime and exists for macOS and Linux only. So ELPI is off by default everywhere, ships first in the CLI and the MCP server, and is missing from the Node, Python and browser packages. Each surface has the same calls as Datalog: a Rust API, bridge ops, five MCP tools and `tm elpi`.

Until lean-elpi 0.4 is on crates.io, the workspace builds against a checkout next to it. The guide (`docs/elpi.md`, on the site as the ELPI guide) has every call, term and error, and `lat.md/elpi.md` records the thirteen decisions, including the ones made by default rather than by interview.
