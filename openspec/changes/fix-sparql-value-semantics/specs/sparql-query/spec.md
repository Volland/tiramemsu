## ADDED Requirements

### Requirement: Exact decimal arithmetic and numeric promotion
Arithmetic, `ABS`, `ROUND`, `CEIL`, `FLOOR`, `SUM` and `AVG` SHALL follow XPath numeric type promotion (integer, then decimal, then float, then double). Operations whose promoted type is `xsd:decimal` SHALL be computed exactly with at least 28 significant digits and SHALL return `xsd:decimal`; `xsd:integer` divided by `xsd:integer` SHALL return `xsd:decimal`; `xsd:float` operands SHALL be numeric and SHALL yield `xsd:float` when no operand is `xsd:double`. A decimal result that exceeds the supported precision SHALL be an expression error. Arithmetic between integers only, or involving a double, SHALL keep its current type.

#### Scenario: Exact decimal sum
- **WHEN** `SELECT ((0.1 + 0.2) AS ?x) WHERE {}` runs
- **THEN** `?x` is `"0.3"^^xsd:decimal`

#### Scenario: Decimal aggregate
- **WHEN** ten statements have the object `0.1` (an `xsd:decimal`) and `SELECT (SUM(?v) AS ?s) WHERE { ?e v:amount ?v }` runs
- **THEN** `?s` is `"1.0"^^xsd:decimal`

#### Scenario: Integer division is decimal
- **WHEN** `SELECT ((1 / 4) AS ?x) WHERE {}` runs
- **THEN** `?x` is `"0.25"^^xsd:decimal`

#### Scenario: Integer arithmetic unchanged
- **WHEN** `SELECT ((2 + 3) AS ?x) WHERE {}` runs
- **THEN** `?x` is `5` as `xsd:integer`

#### Scenario: Decimal overflow
- **WHEN** `SELECT ?x WHERE { BIND((99999999999999999999999999999.0 * 10.0) AS ?x) }` runs
- **THEN** one row is returned with `?x` unbound

### Requirement: String functions preserve language tags
`UCASE`, `LCASE`, `SUBSTR`, `STRBEFORE`, `STRAFTER` and `REPLACE` SHALL return a literal with the language tag of their first argument when that argument is language-tagged, and `CONCAT` SHALL return a language-tagged literal when all its arguments carry the same tag and a plain literal otherwise, as SPARQL 1.1 specifies. `STRBEFORE` and `STRAFTER` with incompatible arguments SHALL be an expression error. The tag SHALL be visible to `LANG`, `LANGMATCHES`, equality and result encoding.

#### Scenario: UCASE keeps the tag
- **WHEN** `(v:x v:label "chat"@fr)` is live and `SELECT (UCASE(?l) AS ?u) (LANG(UCASE(?l)) AS ?t) WHERE { v:x v:label ?l }` runs
- **THEN** `?u` is `"CHAT"@fr` and `?t` is `"fr"`

#### Scenario: CONCAT with mixed tags
- **WHEN** `SELECT (CONCAT("a"@en, "b"@fr) AS ?c) WHERE {}` runs
- **THEN** `?c` is the plain literal `"ab"`

### Requirement: Zoned dates in comparisons
An `xsd:date` with a timezone SHALL keep its timezone in query results. Equality, ordering and `ORDER BY` on dates SHALL compare the date's starting instant, a date without a timezone being compared as if it were UTC. `DATATYPE` of a zoned date SHALL be `xsd:date`.

#### Scenario: Timezone kept in results
- **WHEN** `(v:d v:on "2006-08-24Z"^^xsd:date)` is inserted and `SELECT ?o WHERE { v:d v:on ?o }` runs
- **THEN** `?o` is `"2006-08-24Z"^^xsd:date`

#### Scenario: Zoned and zone-less dates compare by instant
- **WHEN** `(v:a v:on "2006-08-23"^^xsd:date)` and `(v:b v:on "2006-08-23+00:00"^^xsd:date)` are live and `SELECT ?s WHERE { ?s v:on ?o FILTER(?o = "2006-08-23Z"^^xsd:date) }` runs
- **THEN** both `v:a` and `v:b` are returned
