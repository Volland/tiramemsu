---
type: Constraint
---
# Constraints

External rules and project-level commitments that bind several decisions. Each names its source and what it constrains.

## Same Logical Database

Every backend stores the same integers for the same history: statement ids, both clocks, terms and counters, because the paper's guarantees and the differential tests are stated over those rows.

- constrains:: [[backends#Typed Storage Trait]] {strength: must, source: "paper/layered-bitemporal-graphs.tex"}
- constrains:: [[backends#Same Logical Database]] {strength: must, source: "paper/layered-bitemporal-graphs.tex"}

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

## Never Forget

Nothing is deleted or rewritten: corrections are supersedes and forgetting is a retraction that stays in history ([[time-model#Never Forget]]).

- constrains:: [[reasoning#Read-Only And Materialized]] {strength: must, source: "lat.md/time-model.md#Never Forget"}
- constrains:: [[reasoning#Stored Rule Sets]] {strength: must, source: "lat.md/time-model.md#Never Forget"}
- constrains:: [[reasoning#Widening]] {strength: must, source: "lat.md/time-model.md#Never Forget"}

## Single Writer

One writer commits at a time ([[architecture#Connections and Concurrency]]), so work held inside a write transaction delays every other write.

- constrains:: [[reasoning#Explicit Materialization]] {strength: must, source: "lat.md/architecture.md#Connections and Concurrency"}
- constrains:: [[reasoning#Snapshot Then Commit]] {strength: must, source: "lat.md/architecture.md#Connections and Concurrency"}

## Publishable Crates

Every crate published to crates.io must depend only on registry crates; Cargo refuses git and path-only dependencies, optional ones included.

- constrains:: [[reasoning#Eyeron From Crates.io]] {strength: must, source: "cargo publish"}
