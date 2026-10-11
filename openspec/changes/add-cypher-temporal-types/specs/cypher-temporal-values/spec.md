## ADDED Requirements

### Requirement: Temporal value kinds
Cypher SHALL support the value kinds `Date`, `DateTime`, `LocalDateTime`, `Time`, `LocalTime` and `Duration` with openCypher semantics. `DateTime`, `LocalDateTime`, `Time` and `LocalTime` values computed by a query SHALL carry nanosecond precision, and `DateTime` and `Time` values built with a named zone SHALL keep the zone id until they are stored. A `Duration` SHALL hold months, days, seconds and nanoseconds as separate components.

#### Scenario: Sub-millisecond precision in expressions
- **WHEN** `RETURN localtime('12:31:14.645876123') AS t` runs
- **THEN** `t` is LocalTime `12:31:14.645876123`

#### Scenario: Zone id kept in a computed value
- **WHEN** `RETURN datetime({year: 2015, month: 7, day: 21, hour: 20, timezone: 'Europe/Stockholm'}) AS d` runs
- **THEN** `d` renders as `2015-07-21T20:00+02:00[Europe/Stockholm]`

### Requirement: Temporal constructors and projection
The functions `date`, `datetime`, `localdatetime`, `time`, `localtime` and `duration` SHALL construct values from an ISO 8601 string, from a map of components, and (except `duration`) from another temporal value with optional overriding components, as openCypher specifies. `datetime.fromepoch(seconds, nanoseconds)` and `datetime.fromepochmillis(ms)` SHALL construct UTC date-times. An invalid string, an unknown map key or an out-of-range component SHALL fail with `Eval`.

#### Scenario: Construction from a map
- **WHEN** `RETURN date({year: 1984, week: 10, dayOfWeek: 3}) AS d` runs
- **THEN** `d` is Date `1984-03-07`

#### Scenario: Projection with an override
- **WHEN** `RETURN localdatetime({date: date('1984-10-11'), time: localtime('12:00'), hour: 9}) AS x` runs
- **THEN** `x` is LocalDateTime `1984-10-11T09:00`

#### Scenario: Invalid component
- **WHEN** `RETURN date({year: 2025, month: 13, day: 1})` runs
- **THEN** it fails with `Eval`

### Requirement: Temporal truncation
`date.truncate`, `datetime.truncate`, `localdatetime.truncate`, `time.truncate` and `localtime.truncate` SHALL truncate a temporal value to a unit (`millennium`, `century`, `decade`, `year`, `weekYear`, `quarter`, `month`, `week`, `day`, `hour`, `minute`, `second`, `millisecond`, `microsecond`) and apply an optional map of overriding components, as openCypher specifies. A unit that does not apply to the kind SHALL fail with `Eval`.

#### Scenario: Truncate to the week year
- **WHEN** `RETURN date.truncate('weekYear', date('1984-10-11')) AS d` runs
- **THEN** `d` is Date `1984-01-02`

#### Scenario: Truncate with an override
- **WHEN** `RETURN datetime.truncate('day', datetime('2017-01-01T12:31:14Z'), {hour: 10}) AS d` runs
- **THEN** `d` is DateTime `2017-01-01T10:00Z`

### Requirement: Temporal arithmetic
A `Date`, `DateTime`, `LocalDateTime`, `Time` or `LocalTime` plus or minus a `Duration` SHALL yield the same kind, adding months, then days, then seconds, clamping the day of month and following the zone of a zoned value. Two durations SHALL add and subtract component-wise, and a duration SHALL multiply and divide by a number. `duration.between(a, b)` SHALL return the duration from `a` to `b`, and `duration.inMonths`, `duration.inDays` and `duration.inSeconds` the duration in those units, as openCypher specifies. Any other arithmetic on temporal values SHALL fail with `Eval`.

#### Scenario: Month-end clamping
- **WHEN** `RETURN date('2025-01-31') + duration('P1M') AS d` runs
- **THEN** `d` is Date `2025-02-28`

#### Scenario: Duration between two date-times
- **WHEN** `RETURN duration.between(datetime('2026-03-01T00:00Z'), datetime('2026-03-02T12:00Z')) AS d` runs
- **THEN** `d` is Duration `P1DT12H`

#### Scenario: Temporal plus temporal is an error
- **WHEN** `RETURN date('2025-01-01') + date('2025-01-02')` runs
- **THEN** it fails with `Eval`

### Requirement: Temporal comparison and accessors
Values of the same temporal kind SHALL compare by instant (zoned kinds) or by local reading (local kinds); comparing values of different temporal kinds SHALL yield `null`, and `ORDER BY` SHALL follow the openCypher global order. Durations SHALL support only `=` and `<>`, comparing components. Every component accessor of openCypher (`year`, `quarter`, `month`, `week`, `weekYear`, `day`, `ordinalDay`, `dayOfWeek`, `dayOfQuarter`, `hour`, `minute`, `second`, `millisecond`, `microsecond`, `nanosecond`, `timezone`, `offset`, `offsetMinutes`, `offsetSeconds`, `epochSeconds`, `epochMillis`, and the duration components) SHALL be available on the kinds that have it; any other accessor SHALL yield `null`.

#### Scenario: Different kinds are incomparable
- **WHEN** `RETURN date('2025-01-01') < localdatetime('2025-01-02T00:00') AS c` runs
- **THEN** `c` is `null`

#### Scenario: Times compare by instant
- **WHEN** `RETURN time('10:00+02:00') = time('08:00Z') AS e` runs
- **THEN** `e` is `true`

#### Scenario: Week-year accessor
- **WHEN** `RETURN date('2021-01-01').weekYear AS y, date('2021-01-01').week AS w` runs
- **THEN** `y` is 2020 and `w` is 53

### Requirement: Stored temporal values
Writing a temporal value SHALL store a `Date` as `DATE`, a `DateTime` as `DATETIME` with its offset and a `LocalDateTime` as `DATETIME` without a timezone, both truncated to milliseconds and with a zone id resolved to its offset at that instant; a `Time` or `LocalTime` as a `TYPED` `xsd:time` literal and a `Duration` as a `TYPED` `xsd:duration` literal, each in openCypher canonical lexical form. Reading SHALL return stored `xsd:time` literals as `Time` or `LocalTime` and stored `xsd:duration`, `xsd:dayTimeDuration` and `xsd:yearMonthDuration` literals as `Duration`, whatever their lexical form. No storage format change SHALL be required.

#### Scenario: Truncation on store
- **WHEN** `CREATE (:E {at: datetime('2026-03-01T10:00:00.123456789Z')})` runs and `MATCH (e:E) RETURN e.at` reads it back
- **THEN** the value is DateTime `2026-03-01T10:00:00.123Z`

#### Scenario: Duration round trip
- **WHEN** `CREATE (:E {d: duration('PT36H')})` runs and `MATCH (e:E) RETURN e.d` reads it back
- **THEN** the value is Duration `P1DT12H`, stored as `"P1DT12H"^^xsd:duration`

#### Scenario: SPARQL-written duration compared by value
- **WHEN** `(v:e v:d "PT36H"^^xsd:duration)` was inserted through SPARQL, and `MATCH (n) WHERE n.d = duration('P1DT12H') RETURN n` runs
- **THEN** `v:e` is returned

### Requirement: TCK temporal coverage
The openCypher TCK temporal scenarios (`Temporal1` to `Temporal10`, and the temporal cases of `WithOrderBy1` and `WithOrderBy2`) SHALL pass, and the allowlist SHALL NOT contain the umbrella temporal reason. Any scenario that still fails SHALL be listed with a reason that names the specific limitation.

#### Scenario: Umbrella reason removed
- **WHEN** the TCK runner reads the allowlist
- **THEN** no line contains the reason "temporal: durations, time/localtime, map constructors, truncation and printed zone names are out of v1"
