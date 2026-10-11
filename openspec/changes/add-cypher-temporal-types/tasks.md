## 1. Value model

- [ ] 1.1 Add `Val::Time`, `Val::LocalTime`, `Val::Duration` and nanosecond precision plus optional zone id on `DateTime`, `LocalDateTime`, `Time`, `LocalTime` (`crates/tm-cypher/src/value.rs`), with equality, ordering (openCypher global order), `toString` and JSON encoding.
- [ ] 1.2 Conversion to stored values: truncate `DateTime`/`LocalDateTime` to milliseconds, resolve zone ids to offsets, write `Time`/`LocalTime`/`Duration` as `TYPED` `xsd:time`/`xsd:duration` in canonical form; conversion from stored `xsd:time`, `xsd:duration`, `xsd:dayTimeDuration`, `xsd:yearMonthDuration`.
- [ ] 1.3 Add `XSD_TIME`, `XSD_DURATION`, `XSD_YEARMONTH_DURATION` to `crates/tm-core/src/vocab.rs`.

## 2. Functions and operators

- [ ] 2.1 Constructors from string, map and other temporal values (projection with overrides) for `date`, `datetime`, `localdatetime`, `time`, `localtime`, `duration`.
- [ ] 2.2 `.truncate` for all five instant kinds and every unit; `.statement`, `.transaction`, `.realtime` from the facade clock; `datetime.fromepoch`.
- [ ] 2.3 `duration.between`, `duration.inMonths`, `duration.inDays`, `duration.inSeconds`.
- [ ] 2.4 Arithmetic: temporal ± duration, duration ± duration, duration × / number; comparison rules of D5.
- [ ] 2.5 Accessors for every component of every kind, including `weekYear`, `quarter`, `ordinalDay`, `offsetSeconds`, `epochSeconds` and the duration components.
- [ ] 2.6 Register names in `src/funcs.rs`; arity and type checks in `src/sema/check.rs`.

## 3. Store time

- [ ] 3.1 `SET r.validFrom`/`r.validTo` accept `Date`, `LocalDateTime`, `DateTime` (`src/exec/write_set.rs`); other kinds fail with `Eval`.
- [ ] 3.2 Confirm statement time properties stay `DateTime` UTC and work with arithmetic and `duration.between`.

## 4. Tests

- [ ] 4.1 Remove green temporal lines from `crates/tm-cypher/tests/tck/allowlist.txt`; give every residual line a specific reason; the TCK runner passes.
- [ ] 4.2 Unit tests per function family, including DST arithmetic with a named zone, month-end clamping and duration equality.
- [ ] 4.3 Facade tests: stored round trip of each kind (truncation of sub-millisecond digits, zone id resolved to offset), list of times, pattern constant versus `WHERE` value comparison on a SPARQL-written `xsd:duration`.
- [ ] 4.4 Statement time tests: `duration.between(r.validFrom, r.addedAt)`, `r.addedAt - duration('P30D')`, `SET r.validTo = localdatetime(...)`.
- [ ] 4.5 Bridge, Node and Python tests: new kinds arrive as ISO strings.

## 5. Documentation

- [ ] 5.1 `lat.md/query.md#Front Ends#Cypher` (Values, Not in v1) and `#Temporal Syntax`; CHANGELOG behavior changes; `lat check`; validate OpenSpec.
