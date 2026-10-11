## 1. Format and keys

- [ ] 1.1 Storage format bump: create `seal_key(subject, generation, key, provider_ref, created_t, destroyed_t)` with no-delete and destroy-once triggers; set `PRAGMA secure_delete`; no graph row changes; older builds refuse the file.
- [ ] 1.2 `crates/tm-core/src/seal.rs`: AES-SIV encrypt/decrypt with the predicate as associated data, key generation, in-memory key cache with zeroize, `KeyProvider` trait, AES-KW wrapping for the KEK option; cargo feature `sealing` (default on) with `aes-siv` and `zeroize`.
- [ ] 1.3 `OpenOptions::sealing`, `key_encryption_key`, `key_provider` in `crates/tiramemsu/src/db.rs`; `MissingCapability("sealing")` and `KeyUnavailable` errors.

## 2. Schema and sealing on write

- [ ] 2.1 Accept `sys:sensitive true` (`engine/reserved.rs`); never-used check over history and permanence (`engine/schema.rs`); refuse the `sys:unique` combination.
- [ ] 2.2 Seal literal objects of sensitive predicates in `engine/ops.rs#write` (data subject walk, never inline, `lex` = key id prefix + ciphertext); reject IRI and node objects and `TX`-rooted subjects.
- [ ] 2.3 `Tag::Sealed` decoding in sealing-format files (`id.rs`, `codec.rs`, `term.rs`); no plaintext in term caches.

## 3. Erase

- [ ] 3.1 `Tx::erase_subject(subject)`: overwrite or provider-destroy the key, set `destroyed_t`; post-commit `wal_checkpoint(TRUNCATE)` in the facade; `Value::Erased` decoding and rendering (SPARQL/bridge IRI `sys:Erased`, Cypher `null`).
- [ ] 3.2 New key generation for a subject written after erasure.

## 4. Reads

- [ ] 4.1 `tm_unseal` UDF on every connection when sealing is enabled; sealed constants encrypted per bound subject; decrypt-and-compare in Rust for unbound subjects and range filters.

## 5. Derived storage guards

- [ ] 5.1 Text index never indexes `SEALED`; test that recall cannot find a sealed value and that `term_fts` holds no row for it.
- [ ] 5.2 Bundles withhold sealed statements and their referencers; `View::bundle_with` reports withheld eids; JSON bridge `bundle` returns `withheld`.
- [ ] 5.3 Saved answers: sealed result cells stored by reference; `SensitiveConstant` check on save; re-read after erasure shows `sys:Erased`.
- [ ] 5.4 `set_volatile` refuses a sensitive key.
- [ ] 5.5 Expose an `is_sealed(eid)` helper `Tx::set_embedding` calls to refuse embeddings of sealed statements (`SealedValue`); the erase transaction deletes the `embedding` rows of the erased subject's statements, through an exception to `embedding_no_delete` that only the erase transaction can use (decide the mechanism: trigger `WHEN` on a transaction-scoped erase marker, or drop and recreate the trigger inside the transaction); tests that no other path can delete an embedding.

## 6. Bindings

- [ ] 6.1 JSON bridge `eraseSubject`, `sealKeyStatus`, open options `sealing`, `keyEncryptionKey`; Node and Python wrappers; MCP renders `sys:Erased`, no erase tool.

## 7. Tests

- [ ] 7.1 Idempotent assert of the same sealed value; equality query with bound and unbound subject; range filter; annotation of a sensitive fact uses the fact's subject key.
- [ ] 7.2 Erasure: after `erase_subject` and checkpoint, scanning the database file and WAL bytes finds neither the key nor any plaintext of the subject's values; statements, eids, times and history remain; values read as `sys:Erased`; tx metadata records the eraser.
- [ ] 7.3 Flag rules: refused on a used predicate (history included), refused retraction, refused with `sys:unique`.
- [ ] 7.4 Wrapped keys without the KEK give `KeyUnavailable` for sealed values only; external provider destroy and outage paths.
- [ ] 7.5 Never-forget audit still passes; triggers reject deleting a `seal_key` row and a second update.

## 8. Documentation

- [ ] 8.1 `lat.md/time-model.md#Erasure` (implemented, options), `lat.md/data-model.md#ObjectId` and `#Predicate Schema`, `lat.md/storage.md`, roadmap M6 done; `lat check`; validate OpenSpec.
