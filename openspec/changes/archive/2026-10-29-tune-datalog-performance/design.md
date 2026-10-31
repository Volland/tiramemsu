## Context

See proposal.md for the profile and the motivation. The engine is described in [[datalog#Sorted Columnar Relations]] and [[datalog#Interval Sets Through Joins]]:

- Relations are a flat id buffer with hash indexes per bound-position mask.
- Every tuple carries a coalesced interval set.
- Results are decoded and sorted by the store's value order (`terms::total_cmp`), which falls back to comparing `Debug` strings on ties.

[[datalog#Sorted Columnar Relations]] already says that "each rule variant is ordered from actual sizes and re-planned when a delta changes size tenfold". As built, the engine re-plans on every iteration.

Constraints this design must respect:

- **Deterministic results** ([[openspec:datalog-evaluation#Deterministic results]], `constrains` every decision below, strength must): the rows and their order must not change.
- **Provenance on demand** ([[datalog#Provenance On Demand]], strength must): tracked and untracked runs return the same tuples, and a tracked run keeps one derivation per new fragment.
- **Sequential on wasm** ([[datalog#Sequential On Wasm]], strength must): nothing here may need threads.
- **Publishable crates** ([[constraints#Publishable Crates]], strength must): new dependencies must come from crates.io.

## Goals / Non-Goals

**Goals:**
- Cut `goal_rows` to a small part of a closure's time.
- Remove per-derivation allocation, and the interval algebra, where sets cover every instant.
- Remove per-probe locking from the join loop.
- Measure the evaluation and result phases separately.

**Non-Goals:**
- Changing the relation layout to sorted columns, or adding merge, galloping or leapfrog joins (task 4.11 of `add-datalog-engine`).
- Sharing `Value`s between result rows (`Arc<str>`), which would change public types.
- A thread pool for morsels.

## Decisions

### R1 Rank-ordered results
The decisive change. `goal_rows` collects the distinct ids of the relation's rows that survive clipping. It decodes each id once into a table of `(Term, Value)`, sorts the distinct ids with `total_cmp` on their values, and gives each id a rank. Ids with equal values share a rank. Rows are then sorted with a stable sort on `(rank of each column, interval start)` as integers, and the output terms are cloned from the table.

- `motivated_by::` [[constraints#Read Performance First]]
- `constrains` from [[openspec:datalog-evaluation#Deterministic results]]

The order is identical to before:
- Equal ranks mean `total_cmp` returns `Equal`.
- Ties fall back to the stable sort's input order, as they did before.

The work becomes O(d log d) value comparisons for d distinct ids, plus an integer sort of the rows.

`reason` orders its conclusions and relations the same way. Before, it formatted each predicate IRI on every comparison. Now predicates are ranked once by their displayed IRI, and relations by name.

Rejected:
- Sorting by raw id, which breaks the value order.
- Caching `Debug` strings inside `total_cmp`, which is still a string comparison per pair.
- Parallel sorting, which wasm cannot use and which leaves the decoding cost.

### R2 Shortcuts for every-instant sets
`IvSet::union` returns `all()` when either side is all. `subtract` returns empty when the right side is all, and returns the complement directly when the left side is all and the right side is a single interval. `covers` returns true when `self` is all, without allocating.

`Rel::insert` checks `ivs[row].is_all()` before any set work. An existing row whose set is already every instant takes nothing new.

- `motivated_by::` [[constraints#Read Performance First]]

Rejected: a separate time-free evaluator, which would be two kernels to keep equal and contradicts the single-pass design of [[datalog#Interval Sets Through Joins]].

### R3 Drop covered derivations at emit
`Cx::emit` looks up the head tuple in the head relation, which is read-only during a variant. If the row exists and its set covers the derived set, the derivation is not pushed.

Such a derivation adds nothing in `merge` (`insert` returns `None`), so no fact, delta entry or proof is lost. This holds with provenance on too, because `merge` records a derivation only for a new fragment.

- `motivated_by::` [[constraints#Read Performance First]]
- `constrains` from [[datalog#Provenance On Demand]]

Rejected: a per-iteration hash set of emitted tuples. It deduplicates within one iteration only, and `merge` already does that.

### R4 Rows found through a table of row numbers
`Rel::map` becomes a `hashbrown::HashTable<u32>`. It hashes the tuple slice with `FxHasher` and compares entries against `data[row*arity..]`, so no `SmallVec` copy of each tuple is kept. `find` and `insert` hash the slice once. Capacity grows with the table's own amortised doubling.

- `motivated_by::` [[constraints#Read Performance First]]

Rejected:
- A specialised arity-2 relation type: a second implementation for one shape. The table gives most of the memory win for every arity.
- `std` `HashMap<Box<[Id]>, u32>`: still one key allocation per row.

### R5 Access paths resolved once per variant
Before running a rule variant, `eval_rule` walks its order with the bound set and builds, for each positive stored atom, an access resolved from that bound set:
- the loaded predicate (`Arc<Loaded>`, when already loaded),
- the index for the atom's bound-position mask,
- or "probe".

Only accesses that already exist are resolved: `Stored::peek` returns a predicate already loaded, without counting a probe or loading, and `Loaded::built` and `Rel::built` return indexes already built. At run time an access is used only when the probe's predicate and bound mask match the resolved ones. Otherwise the probe goes through `Stored::for_matches` and `Rel::for_rows` as before. That path still counts probes, loads past the threshold, and builds indexes on first use, so its result is picked up by the next variant.

The probe-or-load choice and explain's probe counts are therefore unchanged. This was settled while implementing; the earlier plan of deciding per variant would have shifted loading by one variant.

- `motivated_by::` [[datalog#Probe Or Load Per Atom]]
- `constrains` from [[openspec:datalog-evaluation#Deterministic results]]

Rejected: replacing the `RwLock`s with lock-free maps (still a hash lookup and `Arc` clone per probe).

### R6 Plans cached per variant
`Engine` keeps a cache from `(rule, delta position)` to `(order, sizes)`, where `sizes` are the lengths of the derived relations the rule reads. The cache is cleared per stratum.

A cached plan is reused until one of those relations has grown tenfold since it was planned (or from empty to non-empty), as [[datalog#Sorted Columnar Relations]] already states.

Explain lines are recorded only the first time a variant is planned in a stratum. This fixes the check on `self.iterations`, which is only updated at the end of a stratum.

- `motivated_by::` [[datalog#Sorted Columnar Relations]]

Rejected: freezing plans after iteration 0, as Soufflé does. Relations here start empty, so the first plan is made blind.

### R7 Phase times in explain
`run_goal` measures the time spent in `Engine::run` and in `goal_rows`. Explain appends `time: evaluation <ms> ms` and `time: results <ms> ms`. The benchmark gains an explain-based split column.

Time is read with `std::time::Instant` on non-wasm targets. On `wasm32` targets, which have no monotonic clock in `std`, the lines report 0.

- `motivated_by::` [[datalog#Gates And Oracles]]

Rejected: timing fields on `DatalogResult`. They would make otherwise equal results differ.

## Risks / Trade-offs

- **[Plan reuse picks a worse order mid-stratum]** → The tenfold rule matches the documented intent. Explain still shows each variant's order.
- **[A new dependency, `hashbrown`]** → It is already in the lock file through other crates, and is published on crates.io.
- **[The rank table costs memory for d distinct ids]** → d ≤ rows × arity, and it replaces a `Vec<Value>` sort key per row.

## Migration Plan

Internal only. No storage, API or binding change. Roll back by reverting the commit.
