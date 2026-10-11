## MODIFIED Requirements

### Requirement: Canonical encoding of booleans, dates and date-times
`xsd:boolean` values SHALL be encoded as `BOOL` with payload 1 for true (`"true"`, `"1"`) and 0 for false (`"false"`, `"0"`). `xsd:dateTime` values SHALL be encoded as `DATETIME` with the payload `(epoch_ms << 11) | tz`, where `epoch_ms` is the signed instant in epoch milliseconds and `tz` is 0 for a value without a timezone, or the offset in minutes plus 841 (1 to 1681, covering −14:00 to +14:00). Digits below one millisecond SHALL be truncated; `Z` and `+00:00` are the same offset. A date-time whose instant lies outside ±2^48 ms SHALL be encoded as `TYPED`. Two date-times SHALL be the same term only if both the instant and the offset match. Value comparison of date-times (equality and order) SHALL use the instant `id >> 15` (an arithmetic shift, as SQLite's `>>` is), and a value without a timezone SHALL compare as if it were UTC. `xsd:date` values without a timezone SHALL be encoded as `DATE` with signed days since 1970-01-01 as payload. From the storage format that introduces zoned dates, an `xsd:date` with a timezone SHALL be encoded as `TYPED` with datatype `xsd:date` and the canonical lexical form `YYYY-MM-DD` followed by `Z` for a zero offset or `±HH:MM` otherwise, so `Z` and `+00:00` give one term; files of earlier formats keep the dates they stored, whose timezone was ignored. A date outside the signed 60-bit payload range SHALL be encoded as `TYPED`.

#### Scenario: Offsets are kept as two terms
- **WHEN** `"2026-03-01T12:00:00+02:00"^^xsd:dateTime` and `"2026-03-01T10:00:00Z"^^xsd:dateTime` are encoded
- **THEN** they produce two different `DATETIME` ObjectIds
- **AND** decoding them yields `2026-03-01T12:00:00.000+02:00` and `2026-03-01T10:00:00.000Z`, each with its own offset

#### Scenario: Same instant compares equal
- **WHEN** the two ObjectIds of the previous scenario are compared by value
- **THEN** `id >> 15` is equal for both, computed in Rust and in SQLite

#### Scenario: Date-time without a timezone
- **WHEN** `"2026-03-01T10:00:00"^^xsd:dateTime` is encoded and decoded
- **THEN** its timezone code is 0 and it decodes to `2026-03-01T10:00:00.000` without a timezone
- **AND** its `id >> 15` equals that of `"2026-03-01T10:00:00Z"^^xsd:dateTime`

#### Scenario: Out-of-range date-time
- **WHEN** a date-time whose instant is more than 2^48 ms from the epoch (for example the year 12000) is encoded
- **THEN** it is a `TYPED` ObjectId with datatype `xsd:dateTime` and its lexical form kept verbatim

#### Scenario: Dates before the epoch
- **WHEN** `"1969-12-31"^^xsd:date` is encoded
- **THEN** it is a `DATE` ObjectId with payload −1

#### Scenario: Zoned date keeps its timezone
- **WHEN** `"2006-08-23+00:00"^^xsd:date` and `"2006-08-23Z"^^xsd:date` are encoded in a file of the new format
- **THEN** both produce the same `TYPED` ObjectId whose lexical form is `2006-08-23Z` and whose datatype is `xsd:date`
- **AND** `"2006-08-23"^^xsd:date` is still the inline `DATE` ObjectId

#### Scenario: Boolean forms
- **WHEN** `"1"^^xsd:boolean` and `"true"^^xsd:boolean` are encoded
- **THEN** both produce the `BOOL` ObjectId with payload 1
