## Context

See proposal.md for motivation. The decisions are DL1–DL24 of `lat.md/datalog.md`; this design names them where this change realizes them.

Current state that shapes the approach:

- Reads go through `tm_store::ReadStore` ([[backends#Typed Storage Trait]]): redb implements it natively (`RedbRead`, `RedbWrite`), SQLite through `tm_core::sqlstore::SqlStore`, and the facade's `View` hands either to a closure (`read_store`). The trait offers `scan(Pattern, Scope)`, `count(Pattern, Scope, cap)`, `predicate_stats(p)` (exact live statements, distinct subjects, distinct objects on redb), `statement(eid)`, `added_since(t)`, `retracted_since(t)` and the term dictionary.
- Statements carry `eid, s, p, o, t_add, t_ret, v_from, v_to` with valid time `[v_from, v_to)` in epoch milliseconds, either end open (`tm_core::Valid`).
- `tm-n3` is the template for an optional reasoning crate: pure over facts handed to it, wired through a facade module (`crates/tiramemsu/src/n3.rs`), a bridge module (`bindings/json/src/n3.rs`), MCP tools and CLI subcommands ([[reasoning#Optional Crate]], [[reasoning#Every Surface]]).
- `tm-native` ([[query#Native Runner]]) has the join ideas this kernel reuses (greedy order from exact statistics tried from every first atom, flat rows, generic join for cyclic cores, morsels), but it is bound to the IR's SPARQL and Cypher semantics and to `exec`.
- `add-interval-algebra` task 1 (`Allen`, `Valid::relate`, `Valid::intersects`) is not started; this change starts with it.

## Goals / Non-Goals

**Goals:** a set-at-a-time engine whose hot loop touches only `i64`s; valid time without per-segment re-runs; goal queries that read only what the goal needs; results identical across backends, threads and access choices; no `unsafe`, no git dependency.

**Non-Goals:** writing (change 3), commit-time checks (change 2), query-language calls (change 4); a compiled (code-generating) mode; incremental state kept between runs.

## Constraints

- [[constraints#No Unsafe Code]] constrains every module of `tm-datalog` {strength: must}.
- [[constraints#Publishable Crates]] constrains its dependencies to registry crates {strength: must}: `rustc-hash` and `tm-core`/`tm-store` only; no parser generator with build scripts.
- [[constraints#Same Logical Database]] constrains results: equal on SQLite and redb for the same history {strength: must}.
- [[constraints#Read Performance First]] constrains the kernel design and is the reason for the gates {strength: should}.
- [[query#Query Budgets]]: every loop polls the budget, as the native runner does.

## Decisions

### DL2. Own kernel over ReadStore

`tm-datalog` depends on `tm-core` and `tm-store`. The facade passes it `&dyn ReadStore` plus the view descriptor and budget; the crate never opens a store and never writes.

- motivated_by:: [[reasoning#Optional Crate]] {weight: primary}
- motivated_by:: [[constraints#Publishable Crates]] {weight: secondary}

*Alternatives:* reuse `tm_native::Bgp` (needs `exec`, carries canonical-eid and isomorphism semantics, and only reads the store, not in-memory n-ary relations); extract a shared join crate first (refactors tested hot code before any Datalog exists).

Modules: `syntax` (lexer, parser, AST, spans), `analysis` (safety, stratification, monotonicity, metric and invention recursion, predicate sets), `magic` (adornment and demand rewriting), `plan` (rule variants, join order, access choice), `rel` (columnar relations and indexes), `interval` (interval sets and operators), `eval` (semi-naive loop, aggregates, negation), `source` (store probes and loads, view filtering), `builtin`, `prov` (derivations), `result`.

### DL3. Hand-written parser

A hand-written lexer and recursive-descent parser with byte spans, giving `DatalogParse { line, column, message }`. Prefixed names resolve at parse time against `@prefix` declarations plus the store's reserved prefixes (`rdf:`, `xsd:`, `dl:`).

*Alternatives:* `pest`/`lalrpop` (build-time code generation, extra dependencies, poorer error positions for our needs).

### DL4. Atoms to statements

An atom `p(S, O)` with an IRI predicate not defined by any head is a stored atom: one store pattern `(s?, p, o?)` in the view's scope, deduplicated on `(s, o)` per interval coalescing. `p(S, O) @ E` keeps one tuple per eid. Reserved atoms compile to store access: `triple` to an any-predicate scan, `stmt` and `valid` to `statement(eid)` or a scan carrying the row's fields, `in_graph` to the `sys:inGraph` membership pattern, `added` to the row's `t_add`, `tx_instant` to the dictionary's transaction instant. Values are ObjectIds; constants are looked up once (an unknown constant makes the atom empty, it is not an error).

- motivated_by:: [[reasoning#Statements As IRIs]] {weight: secondary}

### DL5 and DL8. Interval sets as the tuple's time

Every tuple of every relation carries an interval set: a sorted, coalesced `Vec<(i64, i64)>` of half-open intervals with `i64::MIN`/`i64::MAX` as open ends, stored in a side column (one offset per tuple into a fragment arena). Join intersects the sets of its inputs and drops empty results; insertion into a relation unions with the existing set and reports the *new part* as the delta, so semi-naive works per fragment. Negation subtracts, per binding, the negated atom's set from the positive set. Aggregates sweep the endpoints of the group's contributing tuples and emit one value per maximal constant piece. `valid_at(t)` views are the special case of sets `[t, t+1)`. `Atemporal` uses the single set `(−∞, ∞)` and skips interval work.

- motivated_by:: [[reasoning#Pointwise Valid Time]] {weight: primary}

*Alternatives:* N3's segment sweep (cost grows with the global segment count); intersect after the fact (unsound under negation).

### DL6 and DL7. DatalogMTL operators as interval transforms

Each operator is a function on interval sets over the integer timeline with closed operator windows `[a, b]`: `past[a,b]` maps `[f, e)` to `[f+a, e+b)`; `always_past[a,b]` maps `[f, e)` to `[f+b, e+a)` when non-empty (applied to each maximal interval after coalescing); `future` and `always_future` mirror them; `since` and `until` combine two sets by the standard sweep. Head `always_*` operators apply the box transform to the derived set before insertion. Recursion through an operator is detected in the dependency graph; such strata clip every set to `window` before insertion, so the fixpoint is finite (interval endpoints are drawn from a finite set generated by the window and the operator offsets). Saturating arithmetic keeps open ends open.

- motivated_by:: [[datalog#Interval Sets Through Joins]] {weight: primary}

*Alternatives:* dense rational time (conversion and open/closed bounds everywhere); a window for every metric program.

### Allen relations from tm-core

Allen builtins call `tm_core::Valid::relate`, added first (tasks 1.1 and 1.2 of `add-interval-algebra`, done here and checked off there), so the definitions are shared with SPARQL and Cypher.

- motivated_by:: [[constraints#Same Logical Database]] {weight: secondary}

### DL10. Magic sets with per-stratum fallback

Goals with constants adorn the program (bound/free per argument) and add demand predicates (generalized supplementary magic sets). The rewrite is refused per stratum, falling back to that stratum's full evaluation, when it would make a negated or aggregated relation depend on a demand predicate in its own stratum, or when a metric operator separates the bound argument from its consumer. Explain names the fallback.

- motivated_by:: [[constraints#Read Performance First]] {weight: primary}

### DL11. Probe or load per stored atom

For each stored atom of each rule variant, the planner estimates `probes` (how many times it will be looked up with its bound positions: the outer cardinality times the iteration count of its stratum) and `rows` (from `predicate_stats` or a capped `count`). It loads when `probes > rows / 8` or the stratum is recursive and reads the atom in a non-delta position; otherwise it probes the store's permutation for the bound positions. A loaded atom becomes a relation with the index its access pattern needs, built once per run and shared by variants.

- motivated_by:: [[constraints#Read Performance First]] {weight: primary}
- motivated_by:: [[reasoning#Scope And Pruning]] {weight: secondary}

The threshold is a constant to be tuned by the benchmark (task 9).

### DL12. Columnar relations and hybrid joins

A relation of arity k is k `Vec<i64>` columns plus the interval column; indexes are permutation vectors sorted by the needed column order (built lazily per access pattern, Soufflé-style). Rule bodies are planned per semi-naive variant: the delta atom first, then greedily the connected atom with the fewest estimated matches, tried from every first atom as `tm-native` does. Acyclic steps use hash joins (build on the smaller side) or index lookups into a sorted index; a body whose atoms form a cycle over shared variables runs as a leapfrog triejoin over sorted indexes. Delta merge is sort, dedup and coalesce. Variants re-plan when a delta's size changes tenfold.

- motivated_by:: [[backends#Hybrid Join Engine]] {weight: primary}

### DL19. Provenance as a side array

When requested, each inserted fragment records `(rule, premise refs)` where a premise ref is a stored eid or `(relation, tuple index)`, first derivation wins. Proof trees resolve at the end. Untracked runs allocate nothing for it.

### DL21. Invention

`exists Y` binds Y to a run-local invented node keyed by `(rule, frontier values)`, reported as `{invented: n}` in results (materialization, change 3, turns it into a skolem node through `upsert_derived`). Computed head values are interned as literals. Recursion through either is found in analysis and needs `max_new_values`.

### DL23. Parallel morsels, sequential on wasm

Delta tuples are split into morsels of 1 024 above `parallel_threshold` and joined on scoped threads behind the default feature `parallel`; results are merged in morsel order, then sorted, so output is identical. `wasm32` builds without it.

### Result order and decoding

Rows are sorted by the goal variables' ObjectIds' value order (the store's comparison of decoded values) and interval start; terms are decoded only for output. This makes SQLite and redb outputs comparable byte for byte.

### Facade, bridge and surfaces

`crates/tiramemsu/src/datalog.rs` mirrors `n3.rs`: options types, the stored-program lookup in the view (`dl:source`), `store_program` with supersede and the `RuleLanguageConflict` check, and the result conversion. The bridge module `bindings/json/src/datalog.rs` mirrors `n3.rs`. MCP and CLI follow the N3 tools.

## Risks / Trade-offs

- [Interval sets cost memory per tuple] → atemporal runs and `valid_at` views skip the side column; fragments live in one arena.
- [DatalogMTL scope is large for one change] → operators land behind the brute-force oracle from the first day; `since`/`until` last within the temporal group.
- [Magic sets with negation are subtle] → per-stratum fallback plus the property test "demand equals full" on random programs.
- [Probing SQLite per row is slow] → the probe-or-load rule counts probes; the benchmark runs on both backends and tunes the threshold.
- [Gates may be unrealistic] → the gates are recorded in `lat.md/datalog.md` as proposed; missing one is reported in the benchmark README and reviewed, not silently relaxed.
- [Two rule languages confuse users] → `RuleLanguageConflict`; the site explains when to choose which.

## Migration Plan

Additive: a new crate and a feature that is off by default. No storage format change: programs are ordinary statements. Rollback is removing the feature.

## Open Questions

- The probe-or-load threshold constant and the morsel size are tuned by the benchmark; neither changes the specs.
