## REMOVED Requirements

### Requirement: Sensitive flag is reserved
**Reason**: Crypto-shredding is implemented; `sys:sensitive` is now a valid flag in files of the sealing storage format, specified by the `crypto-shredding` capability ("Sensitive predicates").
**Migration**: None for existing files: no format-1 to format-4 file can contain the flag, because every earlier build rejected it. Callers that relied on the `Unsupported` error should open the file with a sealing-capable build.

## ADDED Requirements

### Requirement: Sensitive flag in earlier formats
In files of storage formats earlier than the sealing format, asserting or creating a statement whose predicate is `sys:sensitive` SHALL still fail with `Unsupported { feature }` naming `sys:sensitive`, and the transaction SHALL leave no trace; opening such a file with a sealing-capable build migrates it forward first, after which the flag is accepted.

#### Scenario: Flag refused by a build that cannot seal
- **WHEN** a build compiled without the `sealing` feature asserts `(:email sys:sensitive true)`
- **THEN** the transaction fails with `Unsupported` naming `sys:sensitive`
- **AND** no `tx` row, statement or term from that transaction exists
