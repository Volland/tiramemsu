# Crypto-shredding (M6)

Target release: **0.8 — Trust and erasure**. Milestone M6 (`lat.md/roadmap.md#Milestones`); design: `lat.md/time-model.md#Erasure`; source: `docs/feature-roadmap.md#0.8`.

## Why

Never-forget is the store's central promise, and SQLite triggers enforce it inside the file. Real deployments also hold personal data, and law (GDPR Art. 17 and similar) requires that it can be erased. Without erasure the immutability promise cannot be kept in production: either the data is not stored, or the file is edited by hand and the invariants are broken.

The design has been fixed since format 1: crypto-shredding. Literals of `sys:sensitive` predicates are stored encrypted under a key per data subject, and erasing destroys the key while every row stays. Format 1 reserved the pieces and rejects them today: ObjectId tag 15 `SEALED` (`crates/tm-core/src/id.rs`, `SEALED_FEATURE`), the schema flag `sys:sensitive` (`crates/tm-core/src/vocab.rs`, rejected in `crates/tm-core/src/engine/reserved.rs#check_predicate`), and the table name `seal_key` (`crates/tm-core/src/storage/mod.rs`, `RESERVED_NAMES`).

The hard part is not encryption; it is making sure plaintext of a sealed value never lands anywhere that outlives the key: the FTS5 text index (`term_fts`), the `embedding` table of vector recall, fact bundles, saved-answer records, the volatile table, term caches and the WAL.

## What Changes

- **Storage format bump** (the next free number) creating `seal_key(subject INTEGER NOT NULL, generation INTEGER NOT NULL, key BLOB, provider_ref TEXT, created_t INTEGER NOT NULL, destroyed_t INTEGER, PRIMARY KEY (subject, generation))` with triggers that forbid `DELETE` and allow exactly one update per row, from live to destroyed (key overwritten with NULL, `destroyed_t` set). It also turns on `PRAGMA secure_delete` for the file.
- **`sys:sensitive true`** becomes a valid schema flag. It can be set only on a predicate that no statement has ever used, and it cannot be retracted or superseded, so no plaintext value of a sensitive predicate exists anywhere in history.
- **Sealing on write.** The literal object of a statement whose predicate is sensitive is encrypted with AES-SIV (deterministic, RFC 5297) under the key of its data subject — the node reached by following `STMT` subjects down — and stored as a `SEALED` (tag 15) dictionary term whose `lex` is `<subject ObjectId>.<key generation>:<base64 ciphertext>`; datatype and language tag are inside the ciphertext. A sealed value is never inlined. Same value, same subject, same predicate → same term, so idempotent assert still matches by id. IRI and node objects of a sensitive predicate are rejected (`InvalidTerm`); a subject chain ending in a transaction is `Unsupported`.
- **Keys.** A key is created on the first sealed write for a subject. Three storage options: **in-file** (default; 512-bit key in `seal_key.key`), **in-file wrapped** (the in-file key encrypted under a host-supplied key-encryption key, so a stolen file alone reveals nothing), and **external** through a `KeyProvider` trait (OS keychain, KMS; `seal_key` holds only `provider_ref`). Keys in memory are zeroized on drop.
- **Erase transaction.** `Tx::erase_subject(subject)` destroys the subject's key in an ordinary transaction (so `sys:author`/`sys:reason` record who and why), sets `destroyed_t`, and after commit the facade runs a WAL checkpoint (`TRUNCATE`) so old key pages leave the WAL; with a `KeyProvider`, the provider's `destroy` runs inside the transaction. After erasure every statement, eid, time, provenance and history row stays queryable, and each sealed value of that subject decodes as `Value::Erased` (SPARQL and the bridge render the IRI `sys:Erased`; Cypher returns `null` and keeps the key in `keys()`).
- **Reads.** Sealed values decrypt on decode, on the caller's connection. Equality with a constant uses the subject's key when the subject is bound (index seek) and decrypts candidate rows in Rust otherwise; range filters decrypt in Rust and use no index. Decrypted plaintext is never put in the term caches.
- **Plaintext never leaves the key's reach.** Sealed values are never indexed by `term_fts` and never embedded (`Tx::set_embedding` refuses a statement with a sealed object, and the erase transaction deletes the `embedding` rows of the erased subject's statements, the one exception to that table's no-delete trigger); `View::bundle` withholds sealed statements (and, by the existing closure rule, whatever references them) and reports them; saved answers store sealed result cells as term references re-decoded on read, and `save_answer` refuses a query that compares a sensitive predicate with a literal constant; `set_volatile` refuses a sensitive key; `sys:unique` on a sensitive predicate is refused.
- **Bindings and MCP.** JSON bridge ops `eraseSubject` (transaction op) and `sealKeyStatus` (read), open options `sealing` and `keyEncryptionKey`; Node and Python wrappers; MCP: no erase tool (erasure is an operator action, not an agent action), and query results show `sys:Erased` like any value.

## Capabilities

### New Capabilities

- `crypto-shredding`: sensitive predicates, sealed terms, per-subject keys and their storage options, the erase transaction, and the no-plaintext guarantees for derived storage.

### Modified Capabilities

- `object-encoding`: "ObjectId layout" accepts tag 15 `SEALED` in files of the sealing format.
- `predicate-schema`: "Sensitive flag is reserved" is removed; the flag's behavior moves to `crypto-shredding`.

## Impact

- **`crates/tm-core`:** `src/id.rs` (`Tag::Sealed` decodes in the sealing format), `src/codec.rs` and `src/term.rs` (sealed encode and decode, no plaintext caching), `src/engine/reserved.rs` (accept `sys:sensitive`), `src/engine/schema.rs` (`PredicateSchema.sensitive`, never-used and irreversible checks, `sys:unique` combination refused), `src/engine/ops.rs` (sealing in `write`), `src/engine/volatile.rs` (sensitive key refused), new `src/seal.rs` (AES-SIV, key cache, `KeyProvider` trait, `Value::Erased`), `src/engine/mod.rs` (`Tx::erase_subject`), `src/text.rs` (never index `SEALED`; test-only assertion), `src/bundle.rs` (withhold sealed statements), `src/storage/` (format migration creating `seal_key` and its triggers, `secure_delete`), `src/error.rs` (`SensitiveConstant`, `KeyUnavailable`, `SealedValue`).
- **`crates/tm-exec`:** `src/udf.rs` / `src/decode.rs` (`tm_unseal` decryption UDF registered on every connection when sealing is enabled; comparison of sealed values in Rust), `src/sqlgen/expr.rs` (sealed constants per bound subject).
- **Facade (`crates/tiramemsu`):** `src/db.rs` (`OpenOptions::sealing`, `key_encryption_key`, `key_provider`; post-erase checkpoint), `src/saved.rs` (sealed result cells by reference, constant check), `src/bundle.rs` (withheld report).
- **Bindings:** `bindings/json/src/{tx,read,value}.rs`, Node, Python; `crates/tiramemsu-mcp` renders `sys:Erased` and exposes no erase tool.
- **Dependencies:** `aes-siv` and `zeroize` (RustCrypto) in `tm-core` behind a default-on cargo feature `sealing`; a build without it refuses sealing-format files with `MissingCapability("sealing")`.
- **Coordination:** `add-vector-recall` (format 5, the append-only `embedding` table guarded by `embedding_no_delete`/`embedding_no_update`) must let `Tx::set_embedding` refuse sealed statements, and this change must give the erase transaction a way past `embedding_no_delete` that no other code path can use.
- **Docs:** `lat.md/time-model.md#Erasure`, `lat.md/data-model.md#ObjectId`, `#Predicate Schema`, `lat.md/storage.md`, roadmap M6.

## Non-goals

- Erasing data from backups taken before erasure (documented, not solved).
- Sealing IRIs or node identities; personal data that must be erasable is modelled as literals of sensitive predicates.
- Per-value erasure (one key per data subject only).
- Searchable encryption (blind indexes) for text recall over sealed values.
