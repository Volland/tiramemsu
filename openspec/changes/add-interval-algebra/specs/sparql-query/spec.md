## MODIFIED Requirements

### Requirement: Built-in functions

The system SHALL support these SPARQL 1.1 operators and functions: logical, comparison and arithmetic operators, `IN`/`NOT IN`, `BOUND`, `IF`, `COALESCE`, `sameTerm`, `isIRI`/`isURI`, `isBlank`, `isLiteral`, `isNumeric`, `STR`, `LANG`, `LANGMATCHES`, `DATATYPE`, `IRI`/`URI`, `STRDT`, `STRLANG`, `STRLEN`, `SUBSTR`, `UCASE`, `LCASE`, `STRSTARTS`, `STRENDS`, `CONTAINS`, `STRBEFORE`, `STRAFTER`, `CONCAT`, `ENCODE_FOR_URI`, `REGEX`, `REPLACE`, `ABS`, `CEIL`, `FLOOR`, `ROUND`, `YEAR`, `MONTH`, `DAY`, `HOURS`, `MINUTES`, `SECONDS`, `TIMEZONE`, `TZ`, `NOW`, and XSD constructor casts to `xsd:string`, `xsd:integer`, `xsd:decimal`, `xsd:double`, `xsd:boolean`, `xsd:date` and `xsd:dateTime`. It SHALL also support the Tiramemsu interval functions of the `interval-algebra` capability (`tm:before`, `tm:after`, `tm:meets`, `tm:metBy`, `tm:overlaps`, `tm:overlappedBy`, `tm:starts`, `tm:startedBy`, `tm:during`, `tm:contains`, `tm:finishes`, `tm:finishedBy`, `tm:equals`, `tm:intersects`, `tm:allen`, `tm:interval` and `tm:validInterval`). Any other function, including `RAND`, `BNODE`, `UUID`, `STRUUID`, `MD5`, `SHA1`, `SHA256`, `SHA384`, `SHA512`, the SPARQL 1.2 language-direction functions, any other `tm:` IRI used as a function and any other custom function IRI, SHALL fail with `Unsupported { feature }`, where `feature` names the function. `YEAR` to `SECONDS` SHALL read the local time in the date-time's stored offset. `TZ` SHALL return the stored offset as a plain string (`"Z"` for a zero offset, `""` without a timezone). `TIMEZONE` SHALL return it as an `xsd:dayTimeDuration` and SHALL raise an error for a date-time without a timezone, as SPARQL 1.1 requires.

#### Scenario: String functions
- **WHEN** `(v:alice v:name "Alice Smith")` is live and `SELECT (UCASE(STRBEFORE(?n, " ")) AS ?f) WHERE { v:alice v:name ?n }` is run
- **THEN** the single row has `f = "ALICE"`

#### Scenario: Unsupported function is named
- **WHEN** `SELECT (MD5(?n) AS ?h) WHERE { ?p v:name ?n }` is submitted
- **THEN** the request fails with `Unsupported { feature: "MD5" }` before any row is produced

#### Scenario: Custom function is unsupported
- **WHEN** `SELECT ?x WHERE { ?x v:age ?a FILTER(<http://example.org/fn>(?a)) }` is submitted
- **THEN** the request fails with `Unsupported` whose feature names `http://example.org/fn`

#### Scenario: Interval function is supported
- **WHEN** `SELECT ?r WHERE { ?s ?p ?o ~ ?r FILTER(tm:intersects(?r, "2025-03-01"^^xsd:date)) }` is submitted
- **THEN** it runs without an `Unsupported` error and returns the statements valid on the first millisecond of 2025-03-01

#### Scenario: Unknown tm function is unsupported
- **WHEN** `SELECT ?x WHERE { ?x v:age ?a FILTER(tm:nonsense(?a)) }` is submitted
- **THEN** the request fails with `Unsupported` whose feature names `urn:tiramemsu:tm:nonsense`
