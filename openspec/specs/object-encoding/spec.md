# object-encoding Specification

## Purpose
Defines how every value stored in a statement's `eid`, `s`, `p` and `o` is represented as one signed 64-bit ObjectId with a 4-bit low tag, the canonical encoding that makes value equality integer equality, the term dictionary for values that do not fit inline, and the skolem IRIs that give anonymous nodes an exact RDF round trip.

## Requirements

### Requirement: ObjectId layout
Every ObjectId SHALL be a signed 64-bit integer equal to `(payload << 4) | tag`, where `tag` is the low 4 bits and `payload` is the remaining 60 bits. The tag values SHALL be: 0 `IRI`, 1 `NODE`, 2 `BNODE`, 3 `STMT`, 4 `TX`, 5 `INT`, 6 `BOOL`, 7 `DATETIME`, 8 `DATE`, 9 `SHORT_STR`, 10 `STR`, 11 `LANG_STR`, 12 `TYPED`, 13 `DOUBLE`, 14 `DECIMAL`. Tag 15 is reserved as `SEALED` (a crypto-shredded literal, milestone M6); format 1 SHALL reject it with `Unsupported { feature }` naming M6, both when it is decoded and when it is passed to any write operation. Tags `IRI`, `STR`, `LANG_STR`, `TYPED`, `DOUBLE` and `DECIMAL` SHALL carry a term-dictionary id as payload; all other tags SHALL be inline.

#### Scenario: Tag and payload extraction
- **WHEN** the integer 5 is encoded
- **THEN** its ObjectId is `(5 << 4) | 5 = 85`
- **AND** `85 & 15 = 5` identifies the `INT` tag

#### Scenario: Statement and transaction ids are inline
- **WHEN** statement number 42 and transaction number 7 are encoded
- **THEN** their ObjectIds are `(42 << 4) | 3` and `(7 << 4) | 4`
- **AND** no term-dictionary row is created

#### Scenario: Reserved SEALED tag is rejected
- **WHEN** an ObjectId whose low 4 bits equal 15 is decoded or passed to any write operation
- **THEN** the operation fails with `Unsupported { feature }` naming `SEALED` and milestone M6
- **AND** a failed write leaves no trace

### Requirement: Canonical encoding of integers
An `xsd:integer` value (or an integer given directly through the API) within the signed 60-bit range `[-2^59, 2^59 - 1]` SHALL always be encoded inline as `INT` with the value as payload, and SHALL never create a dictionary entry. An `xsd:integer` outside that range SHALL be encoded as `TYPED` with datatype `xsd:integer` and its canonical decimal lexical form (no `+` sign, no leading zeros). Different lexical forms of the same integer SHALL produce the same ObjectId.

#### Scenario: Lexical variants collapse
- **WHEN** `"01"^^xsd:integer`, `"+1"^^xsd:integer` and `"1"^^xsd:integer` are encoded
- **THEN** all three produce the same `INT` ObjectId

#### Scenario: Range boundaries
- **WHEN** the integers `2^59 - 1` and `-2^59` are encoded
- **THEN** both are `INT`
- **AND** encoding `2^59` or `-2^59 - 1` produces a `TYPED` ObjectId whose term has datatype `xsd:integer` and lexical form equal to the canonical decimal

#### Scenario: Derived integer datatypes stay typed
- **WHEN** `"5"^^xsd:int` is encoded
- **THEN** it is a `TYPED` ObjectId with datatype `xsd:int`, distinct from the `INT` ObjectId of `5`

### Requirement: Canonical encoding of booleans, dates and date-times
`xsd:boolean` values SHALL be encoded as `BOOL` with payload 1 for true (`"true"`, `"1"`) and 0 for false (`"false"`, `"0"`). `xsd:dateTime` values SHALL be encoded as `DATETIME` with the payload `(epoch_ms << 11) | tz`, where `epoch_ms` is the signed instant in epoch milliseconds and `tz` is 0 for a value without a timezone, or the offset in minutes plus 841 (1 to 1681, covering −14:00 to +14:00). Digits below one millisecond SHALL be truncated; `Z` and `+00:00` are the same offset. A date-time whose instant lies outside ±2^48 ms SHALL be encoded as `TYPED`. Two date-times SHALL be the same term only if both the instant and the offset match. Value comparison of date-times (equality and order) SHALL use the instant `id >> 15` (an arithmetic shift, as SQLite's `>>` is), and a value without a timezone SHALL compare as if it were UTC. `xsd:date` values SHALL be encoded as `DATE` with signed days since 1970-01-01 as payload; a timezone suffix on a date SHALL be ignored. A date outside the signed 60-bit payload range SHALL be encoded as `TYPED`.

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

#### Scenario: Boolean forms
- **WHEN** `"1"^^xsd:boolean` and `"true"^^xsd:boolean` are encoded
- **THEN** both produce the `BOOL` ObjectId with payload 1

### Requirement: Canonical encoding of strings
A plain string or `xsd:string` literal whose UTF-8 encoding is at most 7 bytes SHALL be encoded inline as `SHORT_STR` (the bytes plus a 4-bit length), including the empty string. A plain string longer than 7 bytes SHALL be encoded as `STR` through the dictionary. A language-tagged string SHALL always be encoded as `LANG_STR` through the dictionary, never as `SHORT_STR`, with its language tag lower-cased and its lexical form kept exactly.

#### Scenario: Seven-byte boundary
- **WHEN** the strings `"abcdefg"` (7 bytes) and `"abcdefgh"` (8 bytes) are encoded
- **THEN** the first is `SHORT_STR` and creates no dictionary row
- **AND** the second is `STR` and has exactly one dictionary row

#### Scenario: Multi-byte characters count as bytes
- **WHEN** `"héllo"` (6 bytes) and `"€€€"` (9 bytes) are encoded
- **THEN** `"héllo"` is `SHORT_STR` and `"€€€"` is `STR`

#### Scenario: Plain and xsd:string are the same value
- **WHEN** `"hello"` and `"hello"^^xsd:string` are encoded
- **THEN** both produce the same ObjectId

#### Scenario: Language tags are case-insensitive
- **WHEN** `"colour"@en-GB` and `"colour"@en-gb` are encoded
- **THEN** both produce the same `LANG_STR` ObjectId, whose term has language `en-gb`
- **AND** `"hi"@en` is `LANG_STR` although its text is shorter than 8 bytes

#### Scenario: Empty and NUL-containing strings round trip
- **WHEN** the empty string and a 3-byte string containing a NUL byte are encoded and decoded
- **THEN** each decodes to exactly the original string

### Requirement: Canonical encoding of doubles, decimals and other datatypes
`xsd:double` values SHALL be encoded as `DOUBLE` and `xsd:decimal` values as `DECIMAL`, both through the dictionary with a canonical lexical form and with the numeric value stored in the term's `num` column (NULL for NaN). A literal of any other datatype SHALL be encoded as `TYPED` with its lexical form kept verbatim and its datatype IRI stored as the term's datatype. A literal whose lexical form is not valid for its datatype SHALL be encoded as `TYPED` with that datatype and its lexical form kept verbatim.

#### Scenario: Equal doubles share an id
- **WHEN** `"1.0"^^xsd:double` and `"1E0"^^xsd:double` are encoded
- **THEN** both produce the same `DOUBLE` ObjectId
- **AND** its term has `num = 1.0`

#### Scenario: Decimal canonical form
- **WHEN** `"1.50"^^xsd:decimal` and `"01.5"^^xsd:decimal` are encoded
- **THEN** both produce the same `DECIMAL` ObjectId with `num = 1.5`

#### Scenario: Unknown datatype
- **WHEN** `"POINT(1 2)"^^geo:wktLiteral` is encoded
- **THEN** it is a `TYPED` ObjectId whose term has lexical form `POINT(1 2)` and datatype equal to the `IRI` ObjectId of `geo:wktLiteral`

#### Scenario: Ill-typed literal is preserved
- **WHEN** `"abc"^^xsd:integer` is encoded
- **THEN** it is a `TYPED` ObjectId with datatype `xsd:integer` and lexical form `abc`, and it decodes back to exactly that literal

### Requirement: Encode-decode round trip
For every tag, decoding the ObjectId produced by encoding a value SHALL return a value equal to the canonical form of the original, and encoding that decoded value again SHALL return the same ObjectId. A value that can be inlined SHALL never receive a dictionary id.

#### Scenario: Round trip for every tag
- **WHEN** one representative value of each of the 15 non-reserved tags is encoded, decoded and re-encoded
- **THEN** each decoded value equals the canonical input
- **AND** each re-encoded ObjectId equals the first ObjectId

#### Scenario: Property test over random values
- **WHEN** randomly generated integers, booleans, dates, date-times, strings, language strings, doubles, decimals and IRIs are round-tripped
- **THEN** every round trip is exact, and no inlineable value appears in the `term` table

### Requirement: Order within a tag
For the `INT` and `DATE` tags, the signed integer order of ObjectIds SHALL equal the order of the values they encode, including negative values, so that SQLite's signed integer comparison orders them correctly. For `DATETIME`, the signed integer order SHALL be the order of the instant, then of the timezone code, so that `id >> 15` gives value order and a range over instants is one contiguous id range.

#### Scenario: Negative and positive integers
- **WHEN** the integers −1000, −1, 0, 1 and 2^59 − 1 are encoded as `INT`
- **THEN** their ObjectIds are in strictly increasing signed order

#### Scenario: Property test on order
- **WHEN** random pairs of values of the same tag among `INT`, `DATE` and `DATETIME` are encoded
- **THEN** for `INT` and `DATE`, `a < b` if and only if `oid(a) < oid(b)` under signed 64-bit comparison
- **AND** for `DATETIME`, the instant of `a` is before that of `b` if and only if `oid(a) >> 15 < oid(b) >> 15`, and `oid(a) < oid(b)` whenever the instant of `a` is earlier

### Requirement: Term dictionary deduplication and immutability
Each value that needs the dictionary SHALL be stored at most once, identified by `(tag, lex, dt, lang)` where absent `dt` and `lang` compare equal to each other. The dictionary SHALL only grow: a term's id and content never change, and term ids SHALL be allocated from the `next_term` counter.

#### Scenario: Same IRI interned twice
- **WHEN** the IRI `https://example.org/alice` is used in two different transactions
- **THEN** both uses produce the same `IRI` ObjectId
- **AND** the `term` table holds exactly one row for it

#### Scenario: Deduplication with absent datatype and language
- **WHEN** the same 20-byte plain string is encoded twice in separate transactions
- **THEN** the second encoding reuses the first term id instead of inserting a second row with NULL `dt` and NULL `lang`

#### Scenario: Same lexical form, different kinds
- **WHEN** the IRI `urn:x:abcdefghij` and the plain string `"urn:x:abcdefghij"` are encoded
- **THEN** they get different ObjectIds with tags `IRI` and `STR`

### Requirement: Lookup without insertion on the read path
Encoding a value for a read (a pattern constant) SHALL only look the value up and SHALL NOT insert into the dictionary. A dictionary value that is not present SHALL be reported as absent, so that any pattern using it matches nothing.

#### Scenario: Unknown IRI in a read
- **WHEN** a lookup uses an IRI that was never written
- **THEN** the lookup returns no statements
- **AND** the `term` table and `next_term` counter are unchanged

### Requirement: Skolem IRIs for anonymous nodes
A `NODE` with payload `n` SHALL be exported as the IRI `urn:tiramemsu:node:<n>` and a `BNODE` with payload `n` as `urn:tiramemsu:bnode:<n>`, with `<n>` in canonical decimal. Encoding an IRI of exactly that form SHALL return the corresponding `NODE` or `BNODE` ObjectId and SHALL NOT create an `IRI` term. An IRI with that prefix but a non-canonical or out-of-range number SHALL be treated as an ordinary IRI.

#### Scenario: Node round trip
- **WHEN** a new node is created with payload 12, exported as an IRI, and that IRI is encoded again
- **THEN** the exported IRI is `urn:tiramemsu:node:12`
- **AND** encoding it returns the original `NODE` ObjectId, with no new dictionary row

#### Scenario: Blank node round trip
- **WHEN** the IRI `urn:tiramemsu:bnode:3` is encoded
- **THEN** it returns the `BNODE` ObjectId with payload 3

#### Scenario: Non-canonical skolem form
- **WHEN** the IRI `urn:tiramemsu:node:007` is encoded
- **THEN** it is an ordinary `IRI` ObjectId from the dictionary
