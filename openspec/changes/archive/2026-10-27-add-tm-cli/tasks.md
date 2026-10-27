## Status

Implemented 2026-10-27. Depends on `add-redb-backend` and `add-native-runner`. See the Resolution in `design.md`.

## 1. Implementation

- [x] 1.1 Create `crates/tiramemsu-cli` with binary `tm` (clap derive), facade with `redb` feature.
- [x] 1.2 `create`, `info` — `@tg: implements:: [[openspec:tm-cli#Create on a chosen backend]]`, `[[openspec:tm-cli#Inspect a file]]`.
- [x] 1.3 `query` with formats, views, explain, timeout — `@tg: implements:: [[openspec:tm-cli#Query a file]]`.
- [x] 1.4 `import`, `export` — `@tg: implements:: [[openspec:tm-cli#Import and export]]`.
- [x] 1.5 `convert`, `compact` — `@tg: implements:: [[openspec:tm-cli#Convert and compact]]`.

## 2. Tests

- [x] 2.1 Integration tests running the binary for every scenario (`tests/cli.rs`, `std::process::Command` on `CARGO_BIN_EXE_tm`), annotated `@tg: verifies::`.

## 3. Documentation and checks

- [x] 3.1 README CLI quick start; crate README; lat.md `api.md` section CLI; `architecture.md` crates; decision D44.
- [x] 3.2 `tg check`, `tg ontology check`, `tg trace tm-cli --gaps`, `openspec validate add-tm-cli`.
