## 1. Envelope and crypto

- [ ] 1.1 Cargo feature `signing` in `crates/tiramemsu` with `ed25519-dalek`; RFC 8785 canonicalisation of the signed object (with RFC test vectors).
- [ ] 1.2 `SignedBundle::sign`, `SignedBundle::verify`, envelope parse and render (`tiramemsu-signed-bundle/1`), key id derivation.

## 2. Import

- [ ] 2.1 `TrustPolicy` and `Tx::import_signed_bundle`: verify before the writer; `SignatureInvalid`, `UntrustedSigner`, `SignatureRequired`.
- [ ] 2.2 Record `(tx sys:source <keyId>)`; `confirm_existing` option in `crates/tm-core/src/engine/bundle.rs`.
- [ ] 2.3 `Db::preview_bundle` accepts a signed bundle and reports the verification result.

## 3. Surfaces

- [ ] 3.1 JSON bridge `signBundle`, `verifyBundle`, `importBundle` options; Node and Python wrappers.
- [ ] 3.2 MCP `--trusted-key`, `--require-signed-bundles`, applied by `import_bundle` and `preview_bundle`.

## 4. Tests

- [ ] 4.1 Round trip sign/verify; one changed byte in the bundle, the key id or `signedAt` fails verification; reordered JSON keys still verify.
- [ ] 4.2 Untrusted key refused with a non-empty trust list; unsigned bundle refused under `require_signature`; neither takes the writer nor burns ids.
- [ ] 4.3 After a signed import, `View::conflicts` and `text_search` evidence list the signer's key id as a source; with `confirm_existing`, reused facts gain a confirmation.
- [ ] 4.4 Cross-binding: Rust signs and Node and Python verify, and the reverse; MCP refuses an unsigned bundle when started with `--require-signed-bundles`.

## 5. Documentation

- [ ] 5.1 `lat.md/data-model.md#Fact Bundles#Bundle Formats`, `lat.md/api.md#MCP Tools`; `lat check`; validate OpenSpec.
