## ADDED Requirements

### Requirement: Sensitive predicates
In files of the sealing storage format, `(p sys:sensitive true)` SHALL mark predicate `p` as sensitive. Asserting the flag SHALL fail with `SchemaConflict` listing every statement ever stored with predicate `p`, live or retracted, when there is any. Retracting or superseding the flag SHALL fail with `Unsupported`. Asserting `sys:unique true` on a sensitive predicate, or `sys:sensitive true` on a unique one, SHALL fail with `Unsupported`. Each failure SHALL leave no trace.

#### Scenario: Marking an unused predicate
- **WHEN** no statement has ever used `v:email` and `(v:email sys:sensitive true)` is asserted
- **THEN** the flag statement is inserted

#### Scenario: Predicate with history is refused
- **WHEN** `(v:alice v:email "a@x.org")` was asserted and later retracted, and `(v:email sys:sensitive true)` is asserted
- **THEN** the transaction fails with `SchemaConflict` listing the retracted statement

#### Scenario: Flag is permanent
- **WHEN** `v:email` is sensitive and the flag statement is retracted
- **THEN** the transaction fails with `Unsupported` and the flag stays live

### Requirement: Sealed values
The literal object of a statement whose predicate is sensitive SHALL be stored as a `SEALED` dictionary term encrypted with AES-SIV under the key of its data subject, with the predicate as associated data; a sealed value SHALL never be inlined. The data subject SHALL be the node reached by following `STMT` subjects down to an `IRI`, `NODE` or `BNODE`. Encrypting the same value for the same subject and predicate SHALL give the same term, so idempotent assert SHALL match it by id. An `IRI`, node or statement object of a sensitive predicate SHALL fail with `InvalidTerm`, and a subject chain ending in a transaction SHALL fail with `Unsupported`.

#### Scenario: Idempotent sealed assert
- **WHEN** `(v:alice v:email "a@x.org")` is asserted twice on a sensitive `v:email`
- **THEN** the second assert returns `Existing` with the first eid
- **AND** the stored object has tag 15 and the term's `lex` does not contain `a@x.org`

#### Scenario: Annotation shares the fact's key
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` and `v:note` is sensitive, and `(e1 v:note "confidential")` is asserted
- **THEN** the note is sealed under `v:alice`'s key

#### Scenario: Node object refused
- **WHEN** `(v:alice v:email v:bob)` is asserted on a sensitive `v:email`
- **THEN** the transaction fails with `InvalidTerm`

### Requirement: Reading sealed values
Reading a sealed value SHALL decrypt it with its subject's key on the caller's connection, in every view, query language and binding. Equality with a constant SHALL use the subject's key when the subject is bound, and SHALL compare decrypted values otherwise; range comparisons SHALL compare decrypted values. Decrypted values SHALL NOT be stored in any term cache. When a key cannot be obtained (a missing key-encryption key or an unavailable key provider), reading that value SHALL fail with `KeyUnavailable` and other values SHALL remain readable.

#### Scenario: Query by value with unbound subject
- **WHEN** `v:alice` and `v:bob` have sealed `v:email` values and `SELECT ?s WHERE { ?s v:email "a@x.org" }` runs
- **THEN** the subject whose email is `a@x.org` is returned

#### Scenario: Missing key-encryption key
- **WHEN** a file with wrapped keys is opened without its key-encryption key and a query reads a sealed value
- **THEN** that read fails with `KeyUnavailable`, and a query that touches no sealed value succeeds

### Requirement: Key storage options
Keys SHALL be stored in one of three ways chosen at open: in the file's `seal_key` table (default), in `seal_key` wrapped under a host-supplied key-encryption key, or outside the file through a `KeyProvider`, in which case `seal_key` SHALL hold only a provider reference. The `seal_key` table SHALL reject deletes and SHALL allow each row exactly one update, from live to destroyed. Keys held in memory SHALL be zeroized when dropped.

#### Scenario: Key rows cannot be deleted
- **WHEN** an external tool runs `DELETE FROM seal_key`
- **THEN** SQLite aborts the statement and every key row remains

#### Scenario: External provider holds the key
- **WHEN** the database is opened with a key provider and a sealed value is written
- **THEN** the new `seal_key` row has a provider reference and no key bytes

### Requirement: Erase transaction
`Tx::erase_subject(subject)` SHALL destroy the subject's current key in an ordinary transaction: the key bytes are overwritten (or the provider's destroy runs) and `destroyed_t` is set to the transaction number. After the commit the facade SHALL checkpoint the WAL with `TRUNCATE`, with `PRAGMA secure_delete` in effect. Afterwards every statement, eid, transaction time, valid time, layer and history row SHALL remain queryable, and each sealed value of that subject SHALL decode as `Value::Erased`, rendered as the IRI `urn:tiramemsu:sys:Erased` in SPARQL, the JSON bridge and N-Triples, and as `null` in Cypher. A later sealed write for the same subject SHALL use a new key.

#### Scenario: Erased value
- **WHEN** `(v:alice v:email "a@x.org")` is sealed, the transaction `erase_subject(v:alice)` commits with `(tx sys:reason "GDPR request")`, and `SELECT ?e WHERE { v:alice v:email ?e }` runs
- **THEN** `?e` is `sys:Erased`, the statement's eid and `t_add` are unchanged, and the erasing transaction carries the reason

#### Scenario: No key or plaintext left in the file
- **WHEN** a subject is erased and the checkpoint has run
- **THEN** the bytes of the database file and its WAL contain neither the destroyed key nor the plaintext of any of the subject's sealed values

#### Scenario: New data after erasure
- **WHEN** `v:alice` was erased and `(v:alice v:email "new@x.org")` is asserted
- **THEN** the value is sealed under a new key and reads back as `new@x.org`, while the old value still reads as `sys:Erased`

### Requirement: No plaintext in derived storage
Plaintext of a sealed value SHALL never be written to the text index, the `embedding` table, a fact bundle, a saved-answer record or the volatile table. The text index SHALL NOT index sealed values; `Tx::set_embedding` SHALL refuse an embedding for a statement whose object is sealed with `SealedValue`, and the erase transaction SHALL delete every `embedding` row of a statement whose subject is the erased subject, as the only path allowed past the `embedding_no_delete` trigger; `View::bundle` SHALL withhold sealed statements and every statement that references them and SHALL report the withheld eids; saved answers SHALL store sealed result cells as term references decoded when read, and `save_answer` SHALL fail with `SensitiveConstant` when the query or its parameters compare a sensitive predicate with a literal constant; `set_volatile` with a sensitive key SHALL fail with `InvalidTerm`.

#### Scenario: Text recall cannot find a sealed value
- **WHEN** the text index is built and `(v:alice v:name "Alice Liddell")` is sealed on a sensitive `v:name`
- **THEN** `text_search("Liddell")` returns no hit for it and `term_fts` has no row for its object

#### Scenario: Embeddings of an erased subject are removed
- **WHEN** `(v:alice v:worksAt v:acme)` has an embedding, `erase_subject(v:alice)` commits, and a raw connection then tries to delete any other embedding row
- **THEN** the erased subject's embedding rows are gone, vector recall no longer scores them, and the raw delete is refused by `embedding_no_delete`

#### Scenario: Sealed statement cannot be embedded
- **WHEN** `Tx::set_embedding` is called for a statement whose object is sealed
- **THEN** it fails with `SealedValue` and nothing is stored

#### Scenario: Bundle withholds sealed statements
- **WHEN** e1 = `(v:alice v:worksAt v:acme)` has a layer `(e1 v:note "x")` with a sensitive `v:note` and `View::bundle(e1)` is built
- **THEN** the bundle contains e1 and not the note, and the export reports the note's eid as withheld

#### Scenario: Saved answer shows erasure
- **WHEN** an answer selecting `?e` from `v:alice v:email ?e` is saved, `v:alice` is erased, and the saved answer is read
- **THEN** its stored result shows `sys:Erased`, and the saved record never contained the plaintext

#### Scenario: Sensitive constant refused in a saved query
- **WHEN** `save_answer` is called with `SELECT ?s WHERE { ?s v:email "a@x.org" }` on a sensitive `v:email`
- **THEN** it fails with `SensitiveConstant` and nothing is saved
