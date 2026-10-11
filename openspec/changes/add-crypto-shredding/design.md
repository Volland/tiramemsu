## Context

`lat.md/time-model.md#Erasure` fixes the shape: sealing of `sys:sensitive` literals, deterministic AES-SIV so assert stays idempotent, a data subject found by following `STMT` subjects down, `seal_key(subject, key, destroyed_t)` as the one table where a value is erased, `PRAGMA secure_delete` plus a WAL checkpoint, an optional `KeyProvider`, `sys:Erased` after erasure, and the limits (backups, IRIs never sealed). Format 1 reserves tag 15, the flag and the table and rejects them (`object-encoding`, "ObjectId layout"; `predicate-schema`, "Sensitive flag is reserved"; `storage-format`, "Format 1 reserves names for later milestones", which already allows only M6's migration to create `seal_key`).

Derived storage that could hold plaintext today: the FTS5 table `term_fts` (format 2, written in `crates/tm-core/src/engine/mod.rs` through `text::index_value` for string objects), saved-answer records (`saved_answer.query`, `params`, `result` — JSON text, format 3), the `volatile` table, bundles (values in JSON), term caches (`TermDict`, `TermReader`), and the WAL. The `embedding` table of `add-vector-recall` (format 5) is planned: caller-supplied vectors keyed by (model, statement eid), append-only under the triggers `embedding_no_delete` and `embedding_no_update`; there are no derived vector tables.

## Goals / Non-Goals

**Goals:**
- After `erase_subject`, no readable copy of that subject's sealed values remains in the file, its WAL, or any derived table.
- Assert, idempotency, cascade, supersede and history work unchanged on sealed statements.
- Single-file default with no external service; stronger options for deployments that need them.

**Non-Goals:**
- Erasing backups or copies made before erasure; protecting a live process's memory; sealing IRIs.

## Decisions

### D1. AES-SIV, deterministic, with associated data

AES-SIV (RFC 5297, AES-256, 512-bit key) gives deterministic authenticated encryption: one plaintext under one key and one associated data value always gives one ciphertext, so the term dictionary deduplicates and idempotent assert matches by id. The associated data is the predicate's ObjectId, so a ciphertext cannot be moved to another predicate. The plaintext is the canonical literal (lexical form, datatype, language tag), so the term row leaks only that the value is sealed and its length class.

*Alternatives:* AES-GCM with a random nonce (non-deterministic: breaks idempotent assert and equality); AES-GCM-SIV with a derived nonce (equivalent, less standard for deterministic use); hashing values for equality plus random-nonce encryption (two columns, and the hash survives erasure, which defeats it for guessable values).

### D2. The flag can only be set on a never-used predicate, and is permanent

If `sys:sensitive` could be set on a predicate with existing statements, their plaintext would stay in history forever; if it could be retracted, later values would be plaintext under a name users believe is protected. So asserting the flag fails with `SchemaConflict` listing every statement ever stored with that predicate (live or retracted), and retracting or superseding it fails with `Unsupported`.

*Alternatives:* re-seal existing values (needs a row rewrite, forbidden); allow but warn (silently breaks the erasure promise).

### D3. One key per data subject, with `lex` naming the key

The data subject is found by following `STMT` subjects down to an `IRI`, `NODE` or `BNODE`; a chain ending in a `TX` is `Unsupported` (transaction metadata is not personal data of a subject). The key id is the subject's ObjectId and the key generation (D7), written as the prefix of the sealed term's `lex`, so decoding needs no extra column and the format-1 `term` table is unchanged.

*Alternatives:* a key per statement (per-value erasure, but no equality and no idempotency across statements); a `term.key` column (DDL change to a format-1 table).

### D4. Key storage: in-file (default), wrapped, or external

- **In-file:** `seal_key.key` holds the raw key. Erasure overwrites it with NULL inside the transaction; `secure_delete` zeroes freed content; a `wal_checkpoint(TRUNCATE)` after commit removes the old page image from the WAL.
- **Wrapped:** `OpenOptions::key_encryption_key` supplies a 256-bit KEK; `seal_key.key` holds the key wrapped with AES-KW. A stolen file without the KEK reveals nothing; erasure is the same overwrite. Opening a wrapped file without the KEK makes sealed values fail with `KeyUnavailable` while everything else works.
- **External:** a `KeyProvider { fetch, create, destroy }` supplied at open; `seal_key` holds `provider_ref` only. Erasure calls `destroy` inside the transaction; if the commit then fails, the key is already gone, which is safe (erasure is monotone) and reported.

*Alternatives:* in-file only (simplest; a backup of the file is a backup of the keys); external only (no single-file deployments).

### D5. Plaintext never reaches derived storage

- **Text index:** `term_fts` indexes only `SHORT_STR`, `STR` and `LANG_STR` objects; a sealed object has tag 15 and is never decoded for indexing. A test asserts no `term_fts` row exists for any sealed statement and that recall cannot find a sealed value.
- **Embeddings:** `Tx::set_embedding` refuses a statement whose object is sealed (`SealedValue`), because an embedding is derived plaintext. Because a vector of any statement about the subject can still reveal information, and as defense in depth for the requirement recorded in `add-vector-recall` (design, Risks), the erase transaction deletes every `embedding` row whose statement has the erased subject as subject (including its sealed statements). `embedding` is append-only, so this is the single exception to its `embedding_no_delete` trigger: the guard is lifted only inside the erase transaction (for example a trigger `WHEN` condition on a transaction-scoped erase marker that only `erase_subject` sets), never by other code, and `embedding_no_update` stays absolute. The exact mechanism is decided in task 5.5.
- **Bundles:** sealed statements are withheld like the other left-out kinds, together with whatever references them; `View::bundle_with` reports the withheld eids. There is no opt-in to export plaintext.
- **Saved answers:** result cells holding sealed values are stored as `{"sealed": <ObjectId>}` and decoded when read, so they show `sys:Erased` after erasure. `save_answer` refuses a query or parameter that compares a sensitive predicate with a literal constant (`SensitiveConstant`), found statically on the lowered IR (a pattern whose predicate is sensitive and whose object is a constant, or a `FILTER` comparing a variable bound only by such a pattern with a constant).
- **Volatile:** `set_volatile` with a sensitive key fails with `InvalidTerm`.
- **Caches:** term caches hold the sealed term row (ciphertext), never the decrypted value.

*Alternatives:* indexing blinded tokens for recall (a dictionary attack recovers short values, and the tokens survive erasure).

### D6. `sys:unique` and sealing do not combine

Uniqueness across subjects needs comparing plaintexts under different keys, which means decrypting every live value of the predicate on each write. Asserting `sys:unique true` on a sensitive predicate, or `sys:sensitive` on a unique one, fails with `Unsupported`.

### D7. After erasure: `Value::Erased`

A sealed term whose key is destroyed decodes as `Value::Erased`, rendered as the IRI `urn:tiramemsu:sys:Erased` in SPARQL, the JSON bridge and N-Triples, and as `null` in Cypher (the key stays in `keys()`). Writing a new sealed value for an erased subject creates a fresh key (a new `seal_key` generation keyed by `(subject, generation)`), so an erased person who returns gets new, independent data.

## Risks / Trade-offs

- [Backups taken before erasure still hold keys] → Documented; the wrapped and external options keep keys out of file backups.
- [Range filters on sealed values are linear] → Documented; sensitive predicates are expected to be small (emails, names, birth dates), and equality with a bound subject seeks.
- [Static detection of sensitive constants in saved queries is conservative] → It refuses rather than stores; the error names the pattern.
- [AES-SIV leaks equality within a subject] → Inherent to deterministic encryption and required for idempotent assert; documented.
- [A KeyProvider outage makes sealed values unreadable] → `KeyUnavailable` per value; the rest of the store keeps working.

## Migration Plan

One storage format bump: creates `seal_key` and its triggers, sets `secure_delete`, changes no graph row. Files without any sensitive predicate pay nothing beyond the empty table. Older builds refuse the migrated file. The sealing feature is default-on; a build without it refuses sealing-format files only once a `seal_key` row exists, and otherwise opens them.

## Open Questions

- Should `erase_subject` also accept a statement (erase the subject that statement seals) for agent-facing convenience?
- Should an erase transaction optionally retract the subject's sensitive statements in the same transaction, so "now" views stop showing `sys:Erased` rows? (Proposed: optional flag, default off, history stays.)
- Is AES-KW the right wrap for the KEK option, or should it be AES-SIV with empty associated data for one primitive throughout?
