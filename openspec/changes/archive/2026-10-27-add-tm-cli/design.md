## Context

Users choose a backend when they create a database and never again: every other command detects it from the file header ([[openspec:backend-selection#Detection by file header]]).

## Decisions

### D-1. One binary, subcommands, clap derive

`tm` uses clap's derive API; each subcommand is a function over the facade, so the CLI holds no engine logic.

### D-2. Output formats

`table` (default on a terminal) prints aligned columns with decoded terms in N-Triples syntax; `json` is SPARQL 1.1 JSON results for SPARQL and the bridge's JSON for Cypher; `csv`/`tsv` follow the SPARQL 1.1 CSV/TSV results formats; `ntriples` for `CONSTRUCT`/`DESCRIBE`.

### D-3. Errors

Errors print the typed error name and message (`BackendMismatch: …`) on stderr and exit 1; usage errors exit 2. `--explain` prints the plan of whichever runner serves the file.

### D-4. Import

`tm import` uses the facade's bulk import session ([[query#Bulk Import]]) in chunks (default 100 000 statements per transaction) and prints a summary.

## Resolution

Implemented 2026-10-27.

- **Facade additions:** `Db::summary()` (backend, format version, live and total statements, dictionary terms, transactions, file size) for `tm info`, `ReadStore::term_count`, and `tm_core::rdf::write_term` for table and TSV cells; `tiramemsu::rdf` is re-exported.
- **No `assert_cmd`:** the tests run the binary with `std::process::Command` on `CARGO_BIN_EXE_tm`, one fewer dependency.
- **Error names** come from the JSON bridge (`BindError::code`), so the CLI, the bindings and MCP name errors alike. An RDF syntax error in `tm import` is `Custom: RDF syntax in <file>: …` (there is no RDF dialect in `Error::Parse`).
- **Export** writes the live statements from `View::triples`, without the engine's `urn:tiramemsu:sys:` predicates, and each named-graph membership as an extra quad. Stored blank nodes and LPG nodes are exported as their `urn:tiramemsu:` IRIs (`tm_core::rdf::render`), so a re-import keeps them distinct and stable but as IRIs, not blank nodes.
- **`--explain`** covers SPARQL; Cypher has no explain entry point in the facade yet and returns `Unsupported`.
- **`tm bench`** belongs to `add-backend-benchmarks`.
