## Context

Valid time is an immutable, half-open pair of epoch-millisecond columns `v_from`, `v_to` on `triple`, NULL meaning unbounded. It reaches queries through the virtual predicates `tm:validFrom`/`tm:validTo` (`crates/tm-exec/src/virtual_pred.rs`, absent when NULL) and Cypher's `r.validFrom`/`r.validTo` (DateTime or `null`). The assert overlap test is `(a.v_from IS NULL OR b.v_to IS NULL OR a.v_from < b.v_to) AND (b.v_from IS NULL OR a.v_to IS NULL OR b.v_from < a.v_to)`. Writes reject empty or inverted intervals (`InvalidPatch`), so every stored interval is proper.

SPARQL rejects every custom function IRI except XSD casts (`cast_of` in `crates/tm-sparql/src/lower/expr.rs`). Cypher functions are a name registry in `crates/tm-cypher/src/funcs.rs`, evaluated in Rust.

## Goals / Non-Goals

**Goals:**
- One definition of each relation, used by SQL codegen, Cypher and Rust.
- Correct, documented behavior at unbounded ends.
- Pushdown to SQL so interval filters cost what the equivalent comparisons cost.

**Non-Goals:**
- Transaction-time intervals, interval storage, interval indexes beyond `valid_p`.

## Decisions

### D1. Half-open intervals with extended endpoints

An interval is `[from, to)` over the extended integers, `from` ∈ {−∞} ∪ ℤ and `to` ∈ ℤ ∪ {+∞}, with `from < to`. Endpoint comparison treats −∞ = −∞ and +∞ = +∞. The relations are Allen's, stated on endpoints: `before` ⇔ a.to < b.from; `meets` ⇔ a.to = b.from; `overlaps` ⇔ a.from < b.from < a.to < b.to; `starts` ⇔ a.from = b.from ∧ a.to < b.to; `during` ⇔ b.from < a.from ∧ a.to < b.to; `finishes` ⇔ b.from < a.from ∧ a.to = b.to; `equals` ⇔ a.from = b.from ∧ a.to = b.to; and the six converses. Exactly one holds for any pair of proper intervals, which a property test checks over all endpoint orderings including infinities. `intersects` ⇔ a.from < b.to ∧ b.from < a.to, identical to assert's overlap test; it is true for `overlaps`, `starts`, `during`, `finishes`, `equals` and their converses.

*Alternatives:* closed intervals (Allen's original; `meets` would need a shared instant, contradicting the store's half-open model); treating −∞ = −∞ as unequal (would make two always-valid facts not `equals`, which surprises).

### D2. Name the strict relation `overlaps`, and give "any overlap" its own name

Allen's `overlaps` is strict and asymmetric; most people asking "did these overlap" mean `intersects`. Both are provided, the documentation leads with `intersects`, and recipes use it.

*Alternatives:* `overlaps` meaning intersection (breaks the standard vocabulary that `tm:allen` returns).

### D3. Operands: statements, interval values, instants

A statement operand (a SPARQL statement IRI or reifier variable bound to one; a Cypher relationship or statement node) contributes its stored valid interval. Valid time is immutable content, so it is read from the statement's row whatever the view; a non-statement IRI or node is a type error. An instant (`xsd:date` at midnight UTC, `xsd:dateTime` at its instant; Cypher Date, DateTime, LocalDateTime read as UTC for local values) is the interval `[t, t + 1 ms)`. Interval values are built with `tm:interval(from, to)` / `tiramemsu.interval(from, to)`, where `tm:unbounded` (SPARQL) or `null` (Cypher) is an open end; an empty or inverted interval is a type error.

In SPARQL an interval value is the literal `"[from,to)"^^tm:Interval` with ISO 8601 UTC instants and `-inf`/`+inf` for open ends, so it can be bound, projected and compared for equality like any literal. In Cypher it is the map `{from, to}` with DateTime or `null` values, so it needs no new value kind.

*Alternatives:* four-argument forms `tm:overlaps(from1, to1, from2, to2)` (no type to carry, but unbound variables would be needed for open ends, and SPARQL errors on unbound arguments); a new Cypher value kind (heavier than a map for no gain).

### D4. SQL expansion, not a UDF

Each function lowers to an IR `Func` that SQL codegen expands into comparisons on the operands' `v_from`/`v_to` with `IFNULL` sentinels for the infinities (`-9223372036854775808` and `9223372036854775807`, outside the 49-bit instant range). When an operand is the eid variable of a triple pattern in the same group, its alias's columns are used directly; otherwise a correlated lookup on `triple` by eid supplies them. An `intersects` with a constant interval on a live predicate pattern becomes a range condition `valid_p` can serve. Errors (non-statement operand, malformed interval literal) follow SPARQL's rule: `FILTER` false, `BIND` unbound.

*Alternatives:* a scalar UDF `tm_allen(...)` (simple, but hides the comparisons from the planner and needs the function on every host, including the WASM host's probe).

### D5. Cypher evaluates in Rust with the same `Valid::relate`

Cypher expressions run in the interpreter, so the functions call `tm_core::Valid::relate` directly. The dialect differential suite runs the same questions in both languages to keep the SQL expansion and the Rust function equal.

## Risks / Trade-offs

- [Users confuse Allen's `overlaps` with intersection] → D2; `tm:allen` returns the relation name so a query can show which one holds.
- [Point instants at millisecond granularity] → Documented: `tm:during(?r, "2025-03-01"^^xsd:date)` asks about the first millisecond of the day; use `tm:interval` for a whole day.
- [Correlated lookups for non-pattern operands cost a seek each] → Same cost as joining `tm:validFrom`; the common case (operand is a pattern's eid) uses the alias directly.

## Migration Plan

Additive; no format change. Queries that used a `tm:` function IRI previously failed with `Unsupported`, so no working query changes meaning.

## Open Questions

- Should transaction-time intervals get the same functions (`tm:txBefore` …, or an axis argument)?
- Should `tm:Interval` literals be accepted as objects in writes (for example as a valid-time patch), or stay query-only values? (Proposed: query-only.)
