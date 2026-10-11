# Cypher temporal types

Target release: **0.7 — Time in every language**. Closes review finding H3 (`docs/project-review.md`); source: `docs/feature-roadmap.md#0.7`.

## Why

Tiramemsu is bitemporal, but its second query language cannot do date arithmetic. Cypher today has `date`, `datetime`, `localdatetime`, `timestamp`, `datetime.fromepochmillis` and `localdatetime.truncate`, and the `Val` model (`crates/tm-cypher/src/value.rs`) holds only `Date`, `DateTime { ms, tz }` and `LocalDateTime`. There is no `time`, `localtime` or `duration`, no construction from maps, no projection between temporal types, no truncation beyond one function, no `+`/`-` with durations and no `duration.between`. Users can write `MATCH TIME RESPECTING`, but not `r.addedAt - duration('P30D')` or `duration.between(r.validFrom, r.addedAt)`.

The TCK shows the size of the gap: **1 064 of the 1 269 lines** of `crates/tm-cypher/tests/tck/allowlist.txt` carry the single reason "temporal: durations, time/localtime, map constructors, truncation and printed zone names are out of v1". By feature file:

| Feature | What it tests | Allowlisted lines |
|---|---|---|
| `Temporal9` | truncation | 322 |
| `Temporal1` | construction from a map | 207 |
| `Temporal3` | projection from other temporal values | 183 |
| `Temporal10` | `duration.between`, `inMonths`, `inDays`, `inSeconds` | 131 |
| `Temporal2` | construction from a string | 51 |
| `WithOrderBy1`, `WithOrderBy2` | ordering of dates, times, local times, date-times | 40 + 25 |
| `Temporal4` | storing temporal values and lists of them | 36 |
| `Temporal8` | arithmetic with durations | 27 |
| `Temporal7` | comparison | 18 |
| `Temporal6` | `toString` | 17 |
| `Temporal5` | component accessors | 7 |

Classified by what they need (a script over the feature files, recorded in `design.md`): 504 lines need only the new types and functions; 442 also need sub-millisecond precision in computed values; 100 print named zones (`…+01:00[Europe/Stockholm]`); 18 store values and read them back at millisecond precision. Every one of the 1 064 runs as a query expression except those 18, so in-memory precision and zone ids decide most of the outcome, not storage.

## What Changes

- **New Cypher value kinds** `Time` (with offset), `LocalTime` and `Duration` (months, days, seconds, nanoseconds), and nanosecond precision plus an optional zone id on in-memory `DateTime`, `LocalDateTime`, `Time` and `LocalTime` values. Stored values keep the store's precision: `DATETIME` and `DATE` are unchanged (milliseconds, offset kept, zone id resolved to its offset), and `Time`, `LocalTime` and `Duration` are stored as `TYPED` `xsd:time` and `xsd:duration` literals in openCypher's canonical lexical form.
- **Functions** with openCypher semantics: `date`, `datetime`, `localdatetime`, `time`, `localtime`, `duration` from a string, a map, or another temporal value (projection, with overrides); the `.truncate` variants of all five types and every unit (millennium to nanosecond, `weekYear`, `quarter`, `week`); `.statement`, `.transaction`, `.realtime` clocks; `datetime.fromepoch`, `datetime.fromepochmillis`; `duration.between`, `duration.inMonths`, `duration.inDays`, `duration.inSeconds`.
- **Operators and accessors:** temporal `±` duration, duration `±` duration, duration `*`/`/` number; comparison and ordering within a type (across types `null`, durations only `=`/`<>`); all component accessors (`year` … `nanosecond`, `week`, `weekYear`, `quarter`, `dayOfWeek`, `ordinalDay`, `timezone`, `offset`, `offsetMinutes`, `offsetSeconds`, `epochSeconds`, `epochMillis`, and the duration components).
- **Store time in Cypher terms:** `r.validFrom`, `r.validTo`, `r.addedAt` and `r.retractedAt` already return `DateTime` values in UTC (not integers, as the roadmap said); with this change they take part in arithmetic and `duration.between`, so "how late did we learn it" is `duration.between(r.validFrom, r.addedAt)`. `SET r.validFrom`/`r.validTo` accept `Date`, `LocalDateTime` and `DateTime` (a `Time`, `LocalTime` or `Duration` is an `Eval` error). `txAdded`/`txRetracted` stay Integers.
- **TCK:** remove the temporal lines that go green from the allowlist; any residual line gets a precise reason instead of the umbrella one. Target: all 1 064.

### Compatibility

- **Behavior changes (documented in the CHANGELOG):** `+` and `-` between a temporal value and a duration, and between two durations, now return values where they failed with `Eval`; `time(...)`, `localtime(...)` and `duration(...)` are now functions where they failed with `Unsupported`; a computed `DateTime` may carry sub-millisecond digits and a zone id when returned without being stored, so its rendering can include more than three fractional digits and a `[Zone/Id]` suffix. Values read from the store render exactly as today.
- **Statement time properties:** unchanged values and types (`DateTime` in UTC). Code that called `.epochMillis` on them keeps working.
- **JSON bridge and bindings:** new kinds encode as ISO 8601 strings, like dates today; Node and Python receive strings, so no binding type changes.
- **No storage format change.** New kinds use existing `TYPED` terms; the SPARQL side is not changed by this change.

## Capabilities

### New Capabilities

- `cypher-temporal-values`: temporal value kinds, constructors, projection, truncation, arithmetic, comparison and accessors in Cypher, and how they map onto stored values.

### Modified Capabilities

- `cypher-read`: "Result value model" gains `Time`, `LocalTime` and `Duration`; "Expressions and built-in functions" gains the temporal functions and no longer lists durations and `time`/`localtime` as unsupported.
- `cypher-temporal-clauses`: statement time properties take part in temporal arithmetic, and `SET` of valid time accepts every date-bearing kind (ADDED requirement).

## Impact

- **`crates/tm-cypher`:** `src/value.rs` (`Val` and `CypherValue` variants, equality, ordering, rendering, JSON), `src/exec/funcs.rs` (constructors, truncation, clocks, `duration.*`), `src/funcs.rs` (registry `SCALARS`), `src/exec/eval.rs` (arithmetic, accessors at lines ~640–760), `src/exec/access.rs` (`temporal_iri` reads unchanged; conversion to and from stored values), `src/exec/write_set.rs` (`SET r.validFrom` with any date-bearing kind), `src/sema/check.rs` (function arity and names).
- **`crates/tm-core`:** `src/value.rs` reads `xsd:time`, `xsd:duration`, `xsd:dayTimeDuration` and `xsd:yearMonthDuration` literals as typed values for Cypher conversion (no encoding change; `vocab.rs` gains `XSD_TIME`, `XSD_DURATION`, `XSD_YEARMONTH_DURATION` constants).
- **Tests:** `crates/tm-cypher/tests/tck/allowlist.txt` (edits only), new unit tests per function family, `crates/tiramemsu/tests` for stored round trips and statement-time arithmetic.
- **Dependencies:** `chrono` and `chrono-tz` are already dependencies of `tm-cypher`; no new crate.
- **Docs:** `lat.md/query.md#Front Ends#Cypher` (Values, Not in v1), `lat.md/query.md#Temporal Syntax`, CHANGELOG entry.

## Non-goals

- No change to SPARQL date handling (see `fix-sparql-value-semantics` for `xsd:date` time zones) and no change to the inline `DATETIME`/`DATE` encodings.
- No sub-millisecond precision in storage, and no stored zone ids.
- No Allen interval functions (see `add-interval-algebra`).
