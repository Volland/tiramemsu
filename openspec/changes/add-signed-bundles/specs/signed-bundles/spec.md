## ADDED Requirements

### Requirement: Signed bundle envelope
A signed bundle SHALL be the JSON object `{"format": "tiramemsu-signed-bundle/1", "bundle", "signature": {"alg": "Ed25519", "publicKey", "keyId", "signedAt", "sig"}}`, where `bundle` is a `tiramemsu-bundle/1` object, `keyId` is `urn:tiramemsu:key:ed25519:` followed by the lower-case hex of the public key, and `sig` is the Ed25519 signature of the RFC 8785 canonical JSON of `{"alg", "bundle", "keyId", "signedAt"}`. Parsing SHALL refuse another format version or a malformed envelope with `InvalidTerm`.

#### Scenario: Key order does not matter
- **WHEN** a signed bundle is re-serialised with its JSON keys in another order and whitespace added
- **THEN** it still verifies with the same key id

#### Scenario: Wrong version refused
- **WHEN** an envelope with `"format": "tiramemsu-signed-bundle/2"` is parsed
- **THEN** parsing fails with `InvalidTerm`

### Requirement: Signing and verification
`SignedBundle::sign(bundle, signing_key, signed_at)` SHALL produce an envelope that `SignedBundle::verify` accepts, returning the signer's key id. Any change to the bundle, `alg`, `keyId`, `publicKey` or `signedAt` SHALL make verification fail with `SignatureInvalid`. The store SHALL NOT persist private keys.

#### Scenario: Tampered statement
- **WHEN** one statement's object in a signed bundle is changed after signing
- **THEN** verification fails with `SignatureInvalid`

### Requirement: Import under a trust policy
`Tx::import_signed_bundle(signed, policy)` SHALL verify the signature before taking the writer and SHALL fail with `SignatureInvalid` for a bad signature and with `UntrustedSigner` when `policy.trusted` is non-empty and does not contain the signer; importing an unsigned bundle under `policy.require_signature` SHALL fail with `SignatureRequired`. None of these failures SHALL take the writer, burn an id or write anything. A verified bundle SHALL be imported with ordinary bundle semantics.

#### Scenario: Untrusted signer
- **WHEN** a bundle signed by key K2 is imported with `trusted = [K1]`
- **THEN** the import fails with `UntrustedSigner` naming K2's key id, and `last_t` and the id counters are unchanged

#### Scenario: Integrity-only policy
- **WHEN** a validly signed bundle is imported with an empty trust list
- **THEN** it is imported and the signer's key id is recorded

### Requirement: Signer recorded as a source
The importing transaction of a signed bundle SHALL carry `(tx sys:source <keyId>)`. With the option `confirm_existing`, every statement the import reuses instead of inserting SHALL receive a `sys:confirmedBy` link to the importing transaction. Conflict inspection and text-recall evidence SHALL therefore list the key id among the sources of the imported and confirmed statements.

#### Scenario: Signer in conflict evidence
- **WHEN** a signed bundle asserting `(v:alice v:age 31)` is imported into a file holding `(v:alice v:age 30)` with overlapping valid time, and `View::conflicts` runs
- **THEN** the evidence of the value 31 lists the signer's key id as a source

#### Scenario: Confirming existing facts
- **WHEN** a signed bundle containing a fact the target already holds is imported with `confirm_existing`
- **THEN** the existing statement gains a `sys:confirmedBy` link to the importing transaction, whose `sys:source` is the signer's key id

### Requirement: Signed bundles on every surface
The JSON bridge SHALL offer `signBundle` and `verifyBundle` and SHALL accept `signature`, `trustedKeys`, `requireSignature` and `confirmExisting` on `importBundle`; Node and Python SHALL wrap them. The MCP server SHALL accept `--trusted-key` (repeatable) and `--require-signed-bundles` and SHALL apply them to its `import_bundle` and `preview_bundle` tools, so an agent cannot relax the policy.

#### Scenario: MCP refuses unsigned bundles
- **WHEN** the MCP server runs with `--require-signed-bundles` and an agent calls `import_bundle` with an unsigned bundle
- **THEN** the call fails with `SignatureRequired` and nothing is written
