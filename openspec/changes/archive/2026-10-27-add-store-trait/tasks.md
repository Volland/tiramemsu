## Status

Implemented 2026-10-27. First of six changes; see `proposal.md` and the Resolution in `design.md`.

## 1. Contract

- [x] 1.1 Create `crates/tm-store` (workspace member, `unsafe_code = "forbid"`), depending on `tm-core`, `lru` and `unicode-normalization`.
- [x] 1.2 Define `StmtRow`, `TermRow`, `VolatileRow`, `EmbeddingRow`, `TextDoc`, `Pattern`, `Scope` and the `ReadStore` / `WriteStore` / `WriteTxn` / `Backend` traits; annotated `@tg: implements:: [[openspec:storage-trait#Typed storage operations]]`.
- [x] 1.3 Make the pure helpers the port needs public in `tm-core` without behaviour change (`engine::reserved`, `EmbeddingModel::{check, prepare}`, `Bundle::import_order`, `BundleSource`/`export_from`); add `Error::Storage`.

## 2. Reference store

- [x] 2.1 `tm_store::mem::MemStore`: statements by eid, live and all `spo`/`pos`/`osp`, terms by id and key, tx by number and instant, the log, meta, `pred_multi`, volatile, embeddings, text postings; snapshot readers by `Arc`; writers copy and swap.
- [x] 2.2 Duplicate-key and second-retraction refusals (`@tg: implements:: [[openspec:storage-trait#Never forget by construction]]`).

## 3. Engine port

- [x] 3.1 Term dictionary over the traits (encoding, interning, committed SLRU, transaction rollback) and the read-side `TermCache`.
- [x] 3.2 `Engine` (`transact`, `speculate`, `speculate_with`, dry run, `derived_write`, burned ids) and `Tx` core — annotated for `One engine for every backend` and `Speculation without nested savepoints`.
- [x] 3.3 Port `ops`, `cascade`, `supersede`, `graph`, `schema`, `config`, `volatile`, `bundle`, `embedding`.
- [x] 3.4 Reads: views, `triples`, `graphs`, `graph_members`, `values`, `events_since`, `dependents`, `bundle`, conflict inspection, evidence — `@tg: implements:: [[openspec:storage-trait#Snapshot reads]]`.

## 4. Tests

- [x] 4.1 Scenario tests for every scenario (`tests/contract.rs` over the reusable `tm_store::contract` checks), each annotated `@tg: verifies::`.
- [x] 4.2 `tests/oracle.rs`: proptest differential against `tm_core::Store` on `tm-rusqlite`.
- [x] 4.3 The `tm-core` suite still passes after the refactors; the oracle covers the operations of its scenario corpus on `MemStore`.

## 5. Documentation and checks

- [x] 5.1 lat.md: `architecture.md` section Storage Backends (with Trait Engine, Reference Store, Text Tokeniser) and the crate list, `openspec: [storage-trait]` frontmatter; `backends.md` and `constraints.md` for D37–D44; Decision Record rows; roadmap. Test specs live in the OpenSpec scenarios (no duplicate `tests.md` leaves).
- [x] 5.2 Decisions as typed `Decision` sections, constraints as `Constraint` sections with `motivated_by` / `constrains` edges.
- [x] 5.3 `cargo test -p tm-store -p tm-core`, clippy, `tg check`, `tg ontology check`, `tg trace storage-trait --gaps` (the two open requirements belong to `move-sqlite-onto-store`), `openspec validate add-store-trait`.
