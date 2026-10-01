## MODIFIED Requirements

### Requirement: ObjectId layout
Every ObjectId SHALL be a signed 64-bit integer equal to `(payload << 4) | tag`, where `tag` is the low 4 bits and `payload` is the remaining 60 bits. The tag values SHALL be: 0 `IRI`, 1 `NODE`, 2 `BNODE`, 3 `STMT`, 4 `TX`, 5 `INT`, 6 `BOOL`, 7 `DATETIME`, 8 `DATE`, 9 `SHORT_STR`, 10 `STR`, 11 `LANG_STR`, 12 `TYPED`, 13 `DOUBLE`, 14 `DECIMAL`. Tag 15 is reserved as `SEALED` (a crypto-shredded literal, milestone M6); format 1 SHALL reject it with `Unsupported { feature }` naming M6, both when it is decoded and when it is passed to any write operation. Tags `IRI`, `STR`, `LANG_STR`, `TYPED`, `DOUBLE` and `DECIMAL` SHALL carry a term-dictionary id as payload; all other tags SHALL be inline. For the allocated tags `NODE`, `BNODE`, `STMT` and `TX`, the high 12 bits of the payload SHALL be the origin and the low 48 bits the counter; format 1 SHALL allocate only origin 0 and SHALL reject an id with a non-zero origin with `Unsupported { feature }` naming the origin, both on input (skolem IRI, `Value`, bundle import) and when passed to any write operation.

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

#### Scenario: Local ids have origin 0
- **WHEN** statement number 42 is allocated in a format 1 file
- **THEN** its payload is 42, its origin (`payload >> 48`) is 0, and its ObjectId is unchanged from the layout without origins

#### Scenario: Foreign origin is rejected
- **WHEN** the skolem IRI of a statement whose payload is `(1 << 48) | 5` is parsed, or that id is passed to `Tx::assert`
- **THEN** the operation fails with `Unsupported { feature }` naming the origin
- **AND** a failed write leaves no trace
