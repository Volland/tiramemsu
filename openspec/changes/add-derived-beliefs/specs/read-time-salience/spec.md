## ADDED Requirements

### Requirement: Salience score
The salience of a node or statement under a policy `{key, half_life, now}` SHALL be `(1 + ln(1 + count)) · 2^(−age / half_life)`, where `count` is the integer volatile value of `(target, key)` (0 when absent, not an integer, or when the view is not the now view), `age` is `max(0, now − last)`, and `last` is the latest of that volatile value's `updated_at` and the newest commit instant of the target's live statements in the view (for a statement target, its own `tm:addedAt`). `now` SHALL default to the view's instant and MAY be fixed by the policy. A target with no counter and no statement in the view SHALL have score 0. Each result SHALL report `score`, `count`, `last` and `age`.

#### Scenario: Recent and frequent ranks higher
- **WHEN** with `now` fixed, node A has count 9 and was last touched one half-life ago, and node B has count 0 and was added at `now`
- **THEN** A's score is `(1 + ln 10) / 2 ≈ 1.65` and B's score is `1.0`

#### Scenario: Past view ignores volatile counts
- **WHEN** salience of A is read on an as-of view
- **THEN** `count` is 0 and `age` is measured from the as-of instant

### Requirement: Salience never writes
Reading salience SHALL NOT insert, retract or update any statement, volatile value or derived record, and SHALL NOT hide any statement from any query.

#### Scenario: No trace of a read
- **WHEN** salience is read for 1 000 targets
- **THEN** `last_t`, the triple table and the volatile table are unchanged

### Requirement: Salience in every surface
`View::salience(targets, &policy)`, the JSON bridge read op `salience`, SPARQL `tm:salience(?x, <key>, half_life)` with the half-life as an `xsd:dayTimeDuration`, and Cypher `tiramemsu.salience(x, {key, halfLife})` SHALL compute the same score for the same target, view and policy, and the query functions SHALL be usable in `FILTER`/`WHERE` and `ORDER BY`.

#### Scenario: Same score in both languages
- **WHEN** salience of a node is computed through SPARQL, Cypher and Rust with the same key, half-life and fixed `now`
- **THEN** all three scores are equal

#### Scenario: Ranking by salience
- **WHEN** `SELECT ?x WHERE { ?x a v:Topic } ORDER BY DESC(tm:salience(?x, v:accessCount, "P7D"^^xsd:dayTimeDuration))` runs
- **THEN** rows are ordered by descending score, ties by the default term order

### Requirement: Atomic access counter
`Tx::increment_volatile(s, key, by)` SHALL add the integer `by` to the volatile value of `(s, key)` (starting from 0 when absent) and set `updated_at` to the transaction instant, in one write, with the subject and key rules of `set_volatile`. An existing non-integer value SHALL fail with `InvalidTerm`.

#### Scenario: Counting accesses
- **WHEN** `increment_volatile(v:topic1, v:accessCount, 1)` runs in three transactions
- **THEN** the volatile value of `(v:topic1, v:accessCount)` is 3 and no statement was created
