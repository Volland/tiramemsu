## ADDED Requirements

### Requirement: View-aware text recall
Text retrieval SHALL honor transaction time, valid time, and graph scope using the same visibility rules as triple scans.

Text recall SHALL include inline short strings and dictionary-backed plain or language-tagged strings; absence of a dictionary row SHALL NOT make an otherwise visible text value unsearchable.

#### Scenario: Inline text
- **WHEN** a visible statement contains a matching short string encoded inline in its ObjectId
- **THEN** it is returned by text recall without inserting a dictionary row for that value

#### Scenario: Retracted text
- **WHEN** a matching statement is retracted and a now query runs
- **THEN** it is absent from now recall but present in an as-of view before retraction

#### Scenario: Graph and valid time
- **WHEN** a query selects a graph and valid instant
- **THEN** every hit satisfies both selections

### Requirement: Auditable deterministic ranking
Each hit SHALL contain its statement id, lexical score, and available evidence components. Ranking SHALL be deterministic under a documented policy with statement id as the final tie-break.

#### Scenario: Missing confidence
- **WHEN** a hit has no confidence layer
- **THEN** the result identifies confidence as absent rather than fabricating a value

#### Scenario: Equal rank
- **WHEN** two visible hits have equal rank components
- **THEN** repeated queries return the same order

### Requirement: Derived index lifecycle
Hosts lacking FTS5 SHALL reject text retrieval with MissingCapability while core graph operations remain available. Migration and index rebuild SHALL preserve every term, statement, and transaction row.

#### Scenario: Host lacks FTS5
- **WHEN** text retrieval is requested on a host without FTS5
- **THEN** a capability error is returned and ordinary lookups still work

#### Scenario: Rebuild
- **WHEN** the derived text index is rebuilt
- **THEN** retrieval results are restored without altering history

#### Scenario: Format migration
- **WHEN** a format-1 file is opened by a build that supports the text index
- **THEN** it is migrated to format 2 with every term, statement and transaction row unchanged, and a format-1 build refuses the migrated file

#### Scenario: Index written around by a host without FTS5
- **WHEN** a host without FTS5 writes string statements to a file whose text index exists
- **THEN** recall reports the index as unavailable until a writer with FTS5 indexes those statements, and never silently omits them

### Requirement: One recall from every surface
The Rust API, the JSON bridge, SPARQL and Cypher SHALL run the same logical retrieval operation, so equal requests return the same hits in the same order, and each call SHALL be one operation under the caller's query budget.

#### Scenario: Query languages agree
- **WHEN** the same words are recalled through `View::text_search`, SPARQL `tm:textMatch` and Cypher `tiramemsu.text.search`
- **THEN** all three return the same statements, scores and ranks

#### Scenario: Budgeted recall
- **WHEN** a recall runs under a budget whose row limit is smaller than its hits, or whose token is cancelled
- **THEN** it fails with `ResultLimitExceeded` or `Cancelled` and returns no partial result
