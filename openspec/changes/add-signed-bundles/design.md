## Context

A bundle is ordered statements with bundle-local ids and values (`crates/tm-core/src/bundle.rs#Bundle`). `BundleFormat` gives it a versioned JSON form whose round trip is exact (`from_json(to_json(b)) == b`). Import is assert semantics in the caller's transaction (`crates/tm-core/src/engine/bundle.rs`); `Db::preview_bundle` runs it as a dry run. Transaction metadata `sys:author`, `sys:source` and `sys:reason` is allowed on a `TX` subject (`crates/tm-core/src/vocab.rs`, `SYS_TX_METADATA`), and both `View::conflicts` and text-recall evidence already read distinct `sys:author`/`sys:source` values of the asserting and confirming transactions.

## Goals / Non-Goals

**Goals:**
- Detect any change to a bundle in transit, and know which key produced it, before writing.
- Record the signer where existing evidence code already looks.
- Keep `tm-core` free of cryptography and keep unsigned bundles working.

**Non-Goals:**
- PKI, revocation, key storage, encrypted bundles.

## Decisions

### D1. Ed25519 over RFC 8785 canonical JSON

Ed25519 is small, deterministic and widely available in every binding language. The signed bytes are the RFC 8785 (JCS) canonical serialisation of `{"alg", "bundle", "keyId", "signedAt"}`, so whitespace and key order do not matter and any implementation can verify. The bundle JSON is already exact and versioned, so canonicalising it is well defined.

*Alternatives:* signing the N-Triples export (canonical RDF needs RDFC-1.0 and blank-node labelling); JWS compact form (base64 of a non-canonical payload; verifiers must keep the exact bytes); COSE (less familiar to the bindings' users).

### D2. Verify in the facade, before the writer

Verification is pure and runs before `transact` takes the writer, so a forged bundle costs no lock and burns no id. `tm-core` only receives an already verified bundle plus the signer's key id to record.

### D3. The signer is a source, recorded on the transaction

The importing transaction gets `(tx sys:source <urn:tiramemsu:key:ed25519:…>)`. New statements have that transaction as `t_add`, so evidence code counts the signer automatically. Statements the import reuses keep their original `t_add`; with `confirm_existing` they get a `sys:confirmedBy` link to the importing transaction, so the signer also counts as a confirming source. Off by default to keep import idempotent at the statement level (a re-import would add another confirmation).

*Alternatives:* a layer on every imported statement (one extra statement per fact, about 150 bytes each); a new `sys:signer` tx predicate (needs an allow-list change and evidence code changes for little gain).

### D4. Policy is explicit per call

`TrustPolicy { trusted, require_signature }`: an empty `trusted` list accepts any valid signature (integrity only); a non-empty list also requires the key to be listed. `require_signature` refuses unsigned bundles. The MCP server takes the policy from its command line, so an agent cannot relax it.

## Risks / Trade-offs

- [Integrity without identity when `trusted` is empty] → Documented; the key id is still recorded, so later review can filter by it.
- [Key ids are long IRIs] → They are dictionary terms, stored once per key.
- [Canonicalisation bugs break verification across implementations] → Test vectors from RFC 8785 plus cross-binding sign/verify tests (Rust signs, Python and Node verify, and back).

## Migration Plan

Additive. No format change. Unsigned bundles keep working unless a caller asks for `require_signature`.

## Open Questions

- Should the change-set envelope of `add-memory-merge` reuse this exact structure (`tiramemsu-signed-changeset/1`)? (Proposed: yes.)
- Should `signedAt` be checked against a freshness window in `TrustPolicy`?
