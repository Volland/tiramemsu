## ADDED Requirements

### Requirement: No vector capability in WebAssembly
The WASM host SHALL keep stock SQLite and SHALL NOT declare `vector` unless a probe on the running runtime establishes it. libSQL vector operations on a WASM database SHALL fail with `MissingCapability("vector")` and the binding's capability list SHALL omit `vector`, while every other operation, including exact vector recall, keeps working.

#### Scenario: Vector request in a worker
- **WHEN** the libSQL approximate tier is requested through the WebAssembly binding
- **THEN** it fails with `MissingCapability` naming `vector` and the database is unchanged, while an exact vector search succeeds

#### Scenario: Native file with vector index reaches the browser
- **WHEN** a file written by the native libSQL host with a vector index is imported into browser storage
- **THEN** it opens, ordinary reads, writes and text recall work, and the vector tables are left untouched
