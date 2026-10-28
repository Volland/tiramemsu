# Putting an N3 reasoner inside a bitemporal memory

*Tiramemsu runs Notation3 rules through eyeron, a general N3 reasoner. Getting a reasoner that knows nothing about time or history to behave inside a store that knows nothing else took a handful of decisions, one dependency problem and a lot of tests. This is how it was done, what it cost and what is still missing.*

---

A reasoner is a function from facts and rules to conclusions. A memory for agents is a store with two clocks, where nothing is deleted and a fact can be retracted without ever having stopped being a fact that was once believed. Joining them looks like a one-line job: load the statements, run the rules, write the conclusions. It is not, because each side has an assumption the other breaks.

The reasoner assumes its facts are simply true. In the memory, every fact has an interval of valid time and a point of transaction time, and "what follows" depends on which instant you ask about. The reasoner returns a conclusion and moves on. The memory wants the conclusion to be a belief that is withdrawn when its grounds are. The reasoner has a clock builtin, `time:localTime`, that reads the machine's time. The memory wants a run over a past state of the store to give the same answer today as it did then.

Tiramemsu's N3 support is the glue that reconciles these. The reasoner is [eyeron](https://github.com/eyereasoner/eyeron), a native Rust N3 reasoner by Jos De Roo. The integration is the crate `tm-n3` and the facade feature `n3`, off by default. The decisions are numbered N1 to N18 in `lat.md/reasoning.md`; the examples below name the tests that prove them, which run on both storage engines (SQLite and redb).

## The shape: a pure crate and a loading facade

`tm-n3` does not touch the store. It takes statements that have already been loaded (an id, a subject, a predicate and an object as values, and a valid interval), rule text and options, and returns conclusions with proofs. The `tiramemsu` facade does the loading, from any view, and the writing, through its normal transaction type.

That split has one practical consequence worth the design effort. Tiramemsu has two write engines, one on SQL and one on a storage trait that redb implements. A reasoner crate that read or wrote the store would have to know both. This one knows neither, so the same reasoning code serves both, and reasoning can run in the core tier without the query engine (`--no-default-features --features n3`).

## What the reasoner sees

Statements in Tiramemsu have ids, and layers (a confidence, a source) are statements about statements. The reasoner needs to see that. Each loaded statement `e` becomes the IRI `urn:tiramemsu:stmt:N`, and when a rule uses the virtual predicate `tm:content`, the reasoner is also given `e tm:content { s p o }`, plus `tm:validFrom` and `tm:validTo` for the bounded ends of its valid time.

The test `layer_on_a_statement` shows what that buys. A statement `alice worksAt acme` carries a confidence layer of 0.9. The rule reads the layer, reads the content through `tm:content`, and concludes that Alice trusts Acme; the conclusion's proof lists the layer and the statement it is about. Quoted formulas alone would not do this: they identify a statement by its content, so two statements with the same content (two episodes of the same job) would collapse into one, and a premise could not be traced back to a statement id. Identity stays the id.

## Loading only what the rules read

A rule set that mentions two predicates should not make the engine read a million statements of a third. The facade analyses the rules and loads only the statements whose predicate appears in some rule body, including backward rules and nested formulas. A variable in a predicate position turns that off for the run, and the result says whether pruning applied (`pruning_skips_unused_predicates`, `variable_predicate_loads_everything`).

A run can also be scoped to the members of named graphs, or to the statements within a number of hops of seed nodes (`graph_scope`, `seed_scope`), so one agent's or one session's beliefs can be reasoned over without the rest.

## Time: facts that were never true together

This is the central decision, N5. A conclusion holds at an instant exactly when it follows from the statements valid at that instant.

Take Alice working at Acme from 2020 to 2022, and Acme located in Berlin from 2024. The rule says that a person who works at a company located in a place works in that place. A reasoner that ignores valid time concludes that Alice works in Berlin. It is true in the sense that the two statements are in the store; it was never true in the world. `facts_never_simultaneously_valid_do_not_combine` asserts there is no conclusion, and `overlap_gives_the_intersection` asserts that when the two intervals do overlap, the conclusion holds exactly on the overlap.

The way to get there is not obvious, because eyeron gives one proof per conclusion. The tempting plan is to run once over everything and intersect the premise intervals afterwards. That is unsound under negation, where the absence of a fact at one instant matters, and incomplete, because a conclusion with two possible proofs reports only one of them. Instead, the run cuts the timeline at every valid-time bound of the loaded statements, runs eyeron once per segment on the statements valid throughout that segment, and merges neighbouring segments only when the conclusion and its support are identical. The segment count is bounded, and past the bound the result is marked incomplete rather than silently truncated (`segment_limit`).

Negation falls out correctly, because each segment is a plain snapshot. `negation_is_evaluated_per_segment` has an employee with no manager until 2023, and the rule "an employee with no manager is unmanaged" concludes it for exactly the period before 2023.

The strongest evidence for the sweep is a property test, `sweep_agrees_with_snapshots`: for random facts with random intervals, a conclusion of the sweep holds at a random instant exactly when the snapshot view at that instant concludes it.

## Time: the clock that must not move

N3 has `time:localTime`, which a reasoner implements by reading the system clock. In a memory that is a trap: run the same rules over the state of the store as of last Tuesday, and you want the answer of last Tuesday, not one that depends on what time it is now.

The integration pins the clock. `now` is the view's transaction instant, taken once per run, and `time:localTime` returns it. `reproducible_clock` runs a rule that records the clock twice on the same past view and gets the same instant both times, equal to the instant of that transaction. The mechanism today is a rewrite: `tm-n3` turns each `time:localTime` premise into a read of a virtual `tm:run tm:now` fact, so the reasoner never consults the machine clock. A cleaner route is coming from the dependency below, and the rewrite is still in place.

Transaction time needs no machinery of its own. The facade reads the view it is given, so an `as_of` run sees only what was believed then (`as_of_reasoning_ignores_later_facts`).

## Rule sets are memory too

A rule set is stored as one statement, `(rule_set n3:source "<N3 text>")`. Editing it is a supersede, so history keeps every version and an `as_of` run reasons with the rules that were believed then, over the facts that were believed then. Nothing about the rules is special-cased; they inherit the memory's versioning by being statements.

A static check classifies each stored rule set when it is read: monotonic, non-monotonic (negation as failure, aggregates), clock-dependent, or untraced-premises (below). That classification decides what may be written.

## Conclusions that fall

Reasoning can answer a question without writing anything. Materialization writes the conclusions as statements, and the point is that they are statements of a particular kind: derived beliefs. Each one is stored with a support, the statements it was derived from, and the support always includes the rule set's own statement.

The consequence is the property that makes memory trustworthy. Retract a premise and the conclusions resting on it are retracted in the same transaction. Edit the rule set and everything it concluded goes at once. A property test, `conclusions_stay_grounded`, runs random sequences of retractions and supersedes against a materialized rule set and checks after each that every live conclusion's support is live.

Three details keep that honest. Materialization reasons on a snapshot outside the writer and takes the single-writer lock only for a short commit; a conclusion whose support was retracted in the meantime is skipped, with everything resting on it, and reported as stale support rather than failing the run. A blank node in a conclusion becomes a node found or created from a deterministic key, so running the rules again reuses it. And an interval that a later run finds longer is widened by supersede, never narrowed: a conclusion that stops holding loses support and is retracted.

## What materialization refuses

Only monotonic rule sets materialize. If a rule says "an employee with no manager is unmanaged" and a manager statement arrives later, the conclusion becomes false, but nothing was retracted, so the support cascade never fires and the stored conclusion would stay standing on a premise that no longer holds. Such rule sets can be run read-only, where each run is simply correct for its snapshot, but they are refused for materialization.

There is a second refused class, named for what it is: untraced premises. `list:map` is monotonic, but eyeron's proof records only the builtin triple, not the facts the builtin read. A conclusion from it would be stored with incomplete support, and would not fall when one of the hidden premises did. Rather than store ungrounded beliefs, the rule set is refused. The conclusions the store cannot hold at all (a list, a quoted formula, a literal subject) are skipped and reported.

## How it was tested against the reasoner itself

Wrapping a reasoner invites the worry that the wrapper changes what it concludes. The conformance test takes eyeron's own example corpus: of 311 examples that have an expected N3 output, 144 pass through the integration's pipeline (facts loaded into a store as statements, the rules passed as text), and a set of about fifty of those was chosen to cover forward and backward rules, recursion, RDFS and OWL rule sets, negation as failure, aggregation, queries, inference fuses and the math, string, list and time builtins. For each, `View::reason` must conclude what eyeron concludes, value for value.

The rest were left out for stated reasons, and the list is in the test folder: 76 have facts the store cannot hold (lists, quoted formulas, blank nodes, literal subjects), 54 expect output it cannot hold, and 34 use root-level quantification that turns facts into axioms. That is the honest boundary of the mapping: statements hold IRIs, nodes, statements and literals, and N3 is richer than that.

## The dependency that nearly stopped the release

The last problem was not about reasoning at all. eyeron was not on crates.io, so the workspace pinned it to a git commit. Cargo refuses to publish a crate that has a git dependency, optional ones included, so `tm-n3` and everything that enables it could not be published. And eyeron's manifest pulled in `ureq`, an HTTP client for its command-line tools, which brought in `ring`, which broke the Linux aarch64 build of the Python wheel.

The fix was upstream. A pull request to eyeron puts the HTTP client behind a feature and adds a `now` option to the reasoner's options. That option would let `time:localTime` read the pinned clock directly; the integration has not switched to it yet and keeps its rewrite, and dropping the rewrite is a planned step of the change that moves the dependency to crates.io. Until it is merged, the workspace pins the branch of that pull request on the project fork, with `default-features = false`. The cost is visible in the release: the facade, the MCP server and the command line, which depend on `tm-n3` even optionally, are still at 0.8.1 on crates.io, and the crates below them are at 0.10.0. When eyeron publishes, that goes away.

## What is missing

Reasoning on every commit was rejected on purpose: it would put a reasoner inside the single writer for every write. Materialization runs when called. Reconciliation of non-monotonic rule sets, which Tiramemsu's Datalog engine has, is still an open follow-up for N3. eyeron keeps one proof per conclusion and segment, so a stored conclusion may rest on one of several possible supports; it is sound, and the next run restores it if that support goes.

The point of the integration is not that N3 can be run over a store. It is that the three things a reasoner does, inferring, withdrawing and replaying, answer to the same two clocks and the same history as everything else in the memory.

*Related: [Reasoning with N3 in Tiramemsu](../n3/), the mini book; [Datalog over time](datalog-temporal-reasoning.html), the other rule language in the same store.*
