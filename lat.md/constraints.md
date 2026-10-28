---
type: Constraint
---
# Constraints

External rules and project-level commitments that bind several decisions. Each names its source and what it constrains.

## Same Logical Database

Every backend stores the same integers for the same history: statement ids, both clocks, terms and counters, because the paper's guarantees and the differential tests are stated over those rows.

- constrains:: [[backends#Typed Storage Trait]] {strength: must, source: "paper/layered-bitemporal-graphs.tex"}
- constrains:: [[backends#Same Logical Database]] {strength: must, source: "paper/layered-bitemporal-graphs.tex"}

## Grounded Memory

A live statement never rests on a retracted one: what refers to a statement, and what was concluded from it, is retracted with it in the same transaction, so memory keeps no conclusion whose grounds are gone.

- constrains:: [[derived-beliefs#Support Links Extend The Referent Relation]] {strength: must, source: "paper/layered-bitemporal-graphs.tex, the ground core and the least grounded retraction"}
- constrains:: [[derived-beliefs#Idempotent Per Support Set]] {strength: must, source: "paper/layered-bitemporal-graphs.tex"}

## Crash Consistency

A crash or power loss never leaves a database that fails to open or violates an invariant; at most the newest committed transactions may be lost, and only when the durability mode allows it.

- constrains:: [[backends#Durability Modes]] {strength: must, source: "SQLite WAL with synchronous = NORMAL, the baseline since M0"}

## Read Performance First

On the redb backend, read and join speed is the priority, ahead of write cost and file size, which are measured and reported rather than optimised first. Transactional guarantees and security are never traded for speed.

- constrains:: [[backends#Hybrid Join Engine]] {strength: should, source: "design interview 2026-10-27"}
- constrains:: [[backends#Snapshot Readers]] {strength: should, source: "design interview 2026-10-27"}
- constrains:: [[backends#Six Live Permutations]] {strength: should, source: "design interview 2026-10-27"}

## No Unsafe Code

The workspace forbids `unsafe_code`; every backend and runner is safe Rust, and dependencies are chosen so that this holds for the crates of this repository.

- constrains:: [[backends#Typed Storage Trait]] {strength: must, source: "Cargo.toml [workspace.lints.rust]"}
