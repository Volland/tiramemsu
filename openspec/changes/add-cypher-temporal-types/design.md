## Context

Cypher expressions are evaluated in Rust by the `tm-cypher` interpreter (`crates/tm-cypher/src/exec/eval.rs`, `exec/funcs.rs`); only patterns lower to the IR. So temporal values live in `Val` and never reach SQL, except as constants in pattern property maps and as written objects. The store's date encodings are fixed by `object-encoding`: `DATE` (days), `DATETIME` (`(epoch_ms << 11) | tz`, milliseconds, offset kept), everything else `TYPED`. Statement time (`v_from`, `v_to`, `tx.instant`) is epoch milliseconds and is exposed through the virtual predicates `tm:validFrom`, `tm:addedAt`, … as `DATETIME` with offset 0, which `Exec::stmt_time` turns into `Val::DateTime { ms, tz: 0 }` (`crates/tm-cypher/src/exec/access.rs`).

The TCK runner (`crates/tm-cypher/tests/tck.rs`) is closed in both directions: an allowlisted line that starts passing fails the run, so the allowlist edit is part of the change.

TCK classification of the 1 064 temporal lines (a script that reads each scenario's query and its example row): 504 need only types and functions; 442 also need sub-millisecond computed values; 59 print named zones; 41 need both; 18 store values and read them back, all at millisecond precision. None of the sub-millisecond or named-zone lines store their values.

## Goals / Non-Goals

**Goals:**
- openCypher temporal semantics for every expression-level operation, so the TCK lines go green.
- A precise, documented boundary between in-memory values and stored values.
- Statement time usable in arithmetic without new syntax.

**Non-Goals:**
- Changing inline encodings or adding a format version.
- Making SPARQL and Cypher temporal functions share one implementation (SPARQL evaluates in SQL; Cypher in Rust).

## Decisions

### D1. In-memory precision is nanoseconds; storage precision is unchanged

`Val::DateTime`, `LocalDateTime`, `Time` and `LocalTime` carry nanoseconds; storing a `DateTime` or `LocalDateTime` truncates to milliseconds, as the canonical encoding already does for literal input. `Time`, `LocalTime` and `Duration` are stored as `TYPED` literals whose lexical form keeps nanoseconds, because the dictionary keeps lexical forms verbatim.

*Alternatives:* millisecond precision everywhere (simplest, leaves 483 TCK lines red); a new inline encoding with more precision (format change, and the 60-bit payload cannot hold nanoseconds with an offset over the full range).

### D2. Zone ids are kept in memory, resolved to offsets on store

A `DateTime` or `Time` built with a named zone keeps the zone id while it is a computed value, so `toString` and the TCK print `+01:00[Europe/Stockholm]`, and arithmetic across DST transitions follows the zone. Storing resolves the zone to its offset at that instant, as `datetime('…[Europe/Kyiv]')` already does today.

*Alternatives:* drop zone ids at construction (today's behavior; leaves 100 lines red and makes DST arithmetic wrong); store zone ids in a `TYPED` literal (would make one instant two different kinds of stored value depending on how it was written).

### D3. New kinds map onto `TYPED` literals in openCypher canonical form

`Time` → `xsd:time` with offset, `LocalTime` → `xsd:time` without, `Duration` → `xsd:duration`, each in the canonical lexical form openCypher's `toString` produces (for example `PT36H` is written `P1DT12H`; `toString` follows openCypher). Reading accepts any lexical form of `xsd:time`, `xsd:duration`, `xsd:dayTimeDuration` and `xsd:yearMonthDuration`. Lists of temporal values store one statement per element, like other lists.

Consequence: a pattern constant `{d: duration('PT36H')}` matches by term, so it finds values written by Cypher but not a SPARQL-written `"PT36H"^^xsd:duration`; `WHERE n.d = duration('PT36H')` compares by value in Rust and finds both. This mirrors how date-times already match by term in patterns and by instant in `WHERE`.

*Alternatives:* canonicalise `xsd:duration` and `xsd:time` in the core codec (changes the term identity of values already stored by SPARQL, which needs a format bump and breaks idempotent assert across versions).

### D4. Statement time stays `DateTime` in UTC

`r.validFrom`, `r.validTo`, `r.addedAt`, `r.retractedAt` keep returning `DateTime` with offset `Z` and millisecond precision. They now take part in `±` with durations and in `duration.between`. `SET r.validFrom = x` accepts `Date` (midnight UTC), `LocalDateTime` (read as UTC, the rule date-times without a zone already follow for ordering) and `DateTime` (its instant); other kinds fail with `Eval`. The roadmap's "integers today" premise was outdated: only `txAdded`/`txRetracted` are Integers, and they stay so because they are transaction numbers, not times.

*Alternatives:* return `LocalDateTime` for valid time (valid time has no zone, but it is an instant in UTC; a local value would compare incorrectly with zoned values).

### D5. Comparison and ordering follow openCypher

Values of the same kind compare by instant (zoned) or local reading (local kinds); values of different temporal kinds are incomparable (`null` in `WHERE`, ordered by kind in `ORDER BY` per the openCypher global order); durations support only `=` and `<>`, comparing months, days, seconds and nanoseconds component-wise.

### D6. Clock functions read the operation's clocks

`.transaction` is the instant the query or transaction started (one value per query); `.statement` equals `.transaction` (there is one statement per query); `.realtime` reads the clock at each call. They use the facade `Clock`, so tests are deterministic.

## Risks / Trade-offs

- [Rendering of computed values changes] → Only values with sub-millisecond digits or zone ids render differently; stored values are unchanged. Called out in the CHANGELOG.
- [Truncation on store loses digits silently] → Documented in the result value model; a stored round trip test shows the truncation.
- [The TCK lines may hide unrelated gaps] → Each line that stays red gets its own reason; the umbrella reason is removed entirely.
- [Duration arithmetic on months is calendar-dependent] → Follow openCypher's rules (months then days then seconds, clamped day of month), tested by `Temporal8`.

## Migration Plan

No data migration and no format change. The change is additive except the behavior changes listed in the proposal, which go in the CHANGELOG. The allowlist shrinks in the same commit as the implementation.

## Open Questions

- Should `USE AS OF` and `USE VALID AT` accept constant temporal expressions such as `datetime() - duration('P7D')`? (Not in this change; the prepass blanks `USE` clauses and would need an expression parser there.)
- Should the JSON bridge tag temporal kinds (`{"$time": "…"}`) so bindings can build native objects? Today all temporal values are plain strings.
