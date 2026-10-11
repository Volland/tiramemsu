# Signed bundles

Target release: **0.8 — Trust and erasure**. Source: `docs/feature-roadmap.md#0.8` ("Signed bundles"). Split out of `add-memory-merge` so that multi-agent exchange gets a provenance root independently of merging; `add-memory-merge` can reuse the same envelope for change sets.

## Why

Fact bundles move a belief with its evidence between agents' files (`View::bundle`, `Tx::import_bundle`, JSON form `tiramemsu-bundle/1` in `crates/tiramemsu/src/bundle.rs#BundleFormat`). Nothing says who produced a bundle or whether it was changed on the way, so an importing agent cannot distinguish its own colleague's knowledge from anything injected into the channel, and conflict review (`View::conflicts`) cannot count the exporting agent as a source. An Ed25519 signature over the bundle's canonical JSON, checked before import and recorded as a source, is the smallest step that gives exchange a provenance root.

## What Changes

- **Envelope `tiramemsu-signed-bundle/1`:** `{"format", "bundle": <tiramemsu-bundle/1>, "signature": {"alg": "Ed25519", "publicKey": <base64url>, "keyId": "urn:tiramemsu:key:ed25519:<hex of the public key>", "signedAt": <epoch ms>, "sig": <base64url>}}`. The signature covers the RFC 8785 canonical JSON of the bundle object together with `alg`, `keyId` and `signedAt`.
- **Signing and verification** in the facade, behind a default-off cargo feature `signing` (`ed25519-dalek`): `SignedBundle::sign(&Bundle, &SigningKey, signed_at)`, `SignedBundle::verify(&self) -> Result<SignerId>`, and `TrustPolicy { trusted: Vec<VerifyingKey>, require_signature: bool }`. The store never holds a private key; the caller supplies it.
- **Import with policy:** `Tx::import_signed_bundle(&SignedBundle, &TrustPolicy)` verifies before writing (a bad signature is `SignatureInvalid`, an unknown key with a non-empty trust list is `UntrustedSigner`), then imports with ordinary bundle semantics and records the signer as transaction metadata `(tx sys:source <keyId>)`, so the signer is counted as a source by conflict review and text-recall evidence. With `confirm_existing`, statements the import reuses get a `sys:confirmedBy` link to the importing transaction, so the signer also counts as a confirming source for facts the target already held. `Db::preview_bundle` accepts a signed bundle and reports the verification result.
- **Unsigned bundles under a requiring policy** fail with `SignatureRequired` before any write.
- **Surfaces:** JSON bridge `signBundle` and `verifyBundle` (reads) and `importBundle` with `signature`, `trustedKeys`, `requireSignature`, `confirmExisting`; Node and Python wrappers; the MCP server gains `--trusted-key <base64url>` (repeatable) and `--require-signed-bundles`, which its `import_bundle` and `preview_bundle` tools apply.

## Capabilities

### New Capabilities

- `signed-bundles`: the signed envelope, signing, verification, trust policy and signer provenance on import.

### Modified Capabilities

None. Unsigned bundle export, import and formats are unchanged (`fact-bundles`).

## Impact

- **Facade (`crates/tiramemsu`):** `src/bundle.rs` (`SignedBundle`, RFC 8785 canonicalisation of the bundle JSON, envelope parse and render), `src/review.rs` (`preview_bundle` with signed input), a `signing` cargo feature; `src/lib.rs` exports.
- **Core (`crates/tm-core`):** `src/engine/bundle.rs` (import option `confirm_existing`, tx metadata write of `sys:source`), `src/error.rs` (`SignatureInvalid`, `UntrustedSigner`, `SignatureRequired`). Core stays free of a crypto dependency: verification happens in the facade before the import call.
- **Bindings:** `bindings/json/src/{read,tx}.rs`, Node, Python (both build with `signing` on).
- **MCP:** `crates/tiramemsu-mcp/src/config.rs` (trust flags), `src/tools.rs` (`import_bundle`, `preview_bundle` honor them).
- **Docs:** `lat.md/data-model.md#Fact Bundles#Bundle Formats`, `lat.md/api.md#MCP Tools`.
- **Not affected:** storage format, bundle membership rules, N-Triples export.

## Non-goals

- No key management, certificate chains, revocation or identity registry (the verified `sys:author` identity item of the roadmap is separate).
- No encryption of bundles.
- No signed audit log export (a separate 0.8 item).
