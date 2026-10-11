## MODIFIED Requirements

### Requirement: Expressions and built-in functions
The system SHALL evaluate literals (integer, float, string, boolean, `null`, list, map), parameters, arithmetic, string concatenation with `+`, comparison, `=~` regular expressions, list indexing and slicing, map projection (`n {.name, k: expr}`), simple and searched `CASE`, and list comprehension over list values (`[x IN list WHERE p | e]`). It SHALL provide these functions with openCypher semantics: `id`, `elementId`, `labels`, `type`, `keys`, `properties`, `startNode`, `endNode`, `coalesce`, `size`, `head`, `last`, `range`, `toString`, `toInteger`, `toFloat`, `toBoolean`, `toLower`, `toUpper`, `trim`, `ltrim`, `rtrim`, `substring`, `replace`, `split`, `left`, `right`, `reverse`, `abs`, `ceil`, `floor`, `round`, `sign`, `sqrt`, `timestamp`, `nodes`, `relationships` and `length`, and the temporal functions of the `cypher-temporal-values` capability: `date`, `datetime`, `localdatetime`, `time`, `localtime`, `duration`, their `.truncate`, `.statement`, `.transaction` and `.realtime` variants, `datetime.fromepoch`, `datetime.fromepochmillis`, `duration.between`, `duration.inMonths`, `duration.inDays` and `duration.inSeconds`. Calling any other function, or using a pattern comprehension, MUST fail with `Unsupported` naming it.

#### Scenario: CASE and coalesce
- **WHEN** `MATCH (n:Person) RETURN n.name, CASE WHEN n.age >= 18 THEN 'adult' ELSE 'minor' END AS k, coalesce(n.nick, n.name) AS shown` runs
- **THEN** every row has `k` in {"adult", "minor"}, and `shown` is the nickname if present, else the name

#### Scenario: List comprehension
- **WHEN** `RETURN [x IN range(1, 5) WHERE x % 2 = 1 | x * x] AS l` runs
- **THEN** `l` is `[1, 9, 25]`

#### Scenario: Unknown function
- **WHEN** `RETURN apoc.text.clean('x')` is compiled
- **THEN** it fails with `Unsupported` naming `apoc.text.clean`

#### Scenario: Pattern comprehension unsupported
- **WHEN** `MATCH (a) RETURN [(a)-->(b) | b.name]` is compiled
- **THEN** it fails with `Unsupported` naming pattern comprehension

#### Scenario: Durations are supported
- **WHEN** `RETURN duration('P1D') + duration('PT12H') AS d` runs
- **THEN** `d` is Duration `P1DT12H` and no `Unsupported` error is raised

### Requirement: Result value model
Stored values SHALL be returned as Cypher values: integers as Integer; booleans as Boolean; doubles and decimals as Float; plain and language-tagged strings as String (the language tag is dropped); dates as Date; datetimes that carry a timezone as DateTime with their stored offset, and datetimes without a timezone as LocalDateTime, both with millisecond precision (offsets are never normalised to UTC); `xsd:time` literals with an offset as Time and without one as LocalTime; `xsd:duration`, `xsd:dayTimeDuration` and `xsd:yearMonthDuration` literals as Duration; and other datatypes as a String holding the lexical form. Values computed by a query and returned without being stored MAY carry nanosecond precision and a zone id, as the `cypher-temporal-values` capability specifies. A node SHALL be returned as a Node value carrying its element id, its labels and its properties. A relationship SHALL be returned as a Relationship value carrying its element id, type, start and end element ids and properties. Lists, maps and paths SHALL be returned structurally. Every value SHALL have a JSON encoding; temporal values SHALL encode as ISO 8601 strings.

#### Scenario: Scalar round trip of types
- **WHEN** the store holds `v:x` with `v:i 7`, `v:f 1.5`, `v:b true`, `v:d "2025-03-01"^^xsd:date`, `v:t "2025-03-01T10:00:00+02:00"^^xsd:dateTime` and `v:s "hé"@fr`, and `MATCH (n) WHERE n = $x RETURN n.i, n.f, n.b, n.d, n.t, n.s` runs
- **THEN** the values are Integer 7, Float 1.5, Boolean true, Date 2025-03-01, DateTime 2025-03-01T10:00:00.000+02:00 and String "hé"

#### Scenario: DateTime equality compares instants
- **WHEN** `v:x` has `v:a "2026-03-01T12:00:00+02:00"^^xsd:dateTime` and `v:b "2026-03-01T10:00:00Z"^^xsd:dateTime`, and `MATCH (n) WHERE n = $x RETURN n.a = n.b AS eq, n.a AS a, n.b AS b` runs
- **THEN** `eq` is true, `a` is DateTime 2026-03-01T12:00:00.000+02:00 and `b` is DateTime 2026-03-01T10:00:00.000Z
- **AND** `MATCH (n {a: datetime('2026-03-01T10:00:00Z')}) RETURN n` returns `v:x`

#### Scenario: Date-time without timezone reads as LocalDateTime
- **WHEN** `v:x` has `v:l "2026-03-01T09:00:00"^^xsd:dateTime` and `MATCH (n) WHERE n = $x RETURN n.l AS l` runs
- **THEN** `l` is LocalDateTime 2026-03-01T09:00:00.000

#### Scenario: Times and durations are typed values
- **WHEN** `v:x` has `v:open "09:30:00+01:00"^^xsd:time`, `v:close "17:00:00"^^xsd:time` and `v:len "PT7H30M"^^xsd:duration`, and `MATCH (n) WHERE n = $x RETURN n.open, n.close, n.len` runs
- **THEN** the values are Time 09:30+01:00, LocalTime 17:00 and Duration PT7H30M

#### Scenario: Node and relationship values
- **WHEN** `MATCH (a:Person)-[r:worksAt]->(c) RETURN a, r` runs for `(v:alice v:worksAt v:acme)` with eid e1
- **THEN** `a` is a Node with element id `urn:tiramemsu:v:alice`, labels `["Person"]` and its properties
- **AND** `r` is a Relationship with type `worksAt`, start element id `urn:tiramemsu:v:alice`, end element id `urn:tiramemsu:v:acme`, and the element id of e1
