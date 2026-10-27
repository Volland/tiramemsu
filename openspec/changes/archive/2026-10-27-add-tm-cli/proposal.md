# The `tm` command-line tool

Target release: **0.8 — Two engines**. Fourth of six changes (D44 in `lat.md/overview.md#Decision Record`). Depends on `add-redb-backend` and `add-native-runner`.

## Why

The only binary today is the MCP server. With two backends a user needs to create a database on the backend of their choice, see what a file is, query it, load data, move it between backends and run the benchmarks, without writing Rust.

## What Changes

- **New crate `tiramemsu-cli`**, binary `tm`, built with the `redb` feature:
  - `tm create <file> [--backend sqlite|redb] [--durability normal|immediate|none]`
  - `tm info <file>`: backend (detected), format version, statements (live and total), terms, transactions, last instant, file size, text-index and embedding state
  - `tm query <file> (--sparql|--cypher) <text|@file> [--as-of <t|instant>] [--valid-at <instant>] [--format table|json|csv|tsv|ntriples] [--explain] [--timeout <ms>]`
  - `tm import <file> <data.nt|.nq|.ttl|.trig> [--chunk <n>]`
  - `tm export <file> [--format nq|nt]`
  - `tm convert <src> <dst> --backend sqlite|redb`
  - `tm compact <file>`
  - `tm bench …` (filled in by `add-backend-benchmarks`)
- The backend is always detected from the file; `--backend` only chooses it for new files and `convert`.
- Exit codes: 0 success, 1 query or data error (typed error name on stderr), 2 usage error.

## Capabilities

### New Capabilities

- `tm-cli`: the commands above.

### Modified Capabilities

None.

## Impact

- **New crate:** `crates/tiramemsu-cli` (clap).
- **Docs:** `README.md` (CLI quick start), `lat.md/api.md` new section CLI, `site/` CLI page.
- **Requirements added:** [[openspec:tm-cli#Create on a chosen backend]], [[openspec:tm-cli#Inspect a file]], [[openspec:tm-cli#Query a file]], [[openspec:tm-cli#Import and export]], [[openspec:tm-cli#Convert and compact]].

## Non-goals

- No server mode (the MCP server covers that); no interactive shell in this change.
