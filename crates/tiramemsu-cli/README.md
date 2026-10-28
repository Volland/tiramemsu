# tiramemsu-cli

`tm`, the command line for [tiramemsu](https://github.com/Volland/tiramemsu) files. You choose a file's storage backend once, when you create it. Every other command detects the backend from the file header and runs on that backend's runner: generated SQL on SQLite, the native runner on redb.

```sh
cargo install tiramemsu-cli

tm create memory.redb --backend redb          # or sqlite (the default)
tm import memory.redb people.ttl events.nq    # N-Triples, N-Quads, Turtle, TriG
tm info memory.redb                           # backend, format version, counts, size
tm query memory.redb --sparql 'SELECT ?p ?o WHERE { <urn:alice> ?p ?o }'
tm query memory.redb --cypher 'MATCH (a)-[r]->(b) RETURN count(*) AS n' --format json
tm query memory.redb --sparql @q.rq --as-of 120 --valid-at 1767225600000
tm query memory.redb --sparql 'SELECT ?s WHERE { ?s <urn:p> <urn:o> }' --explain
tm export memory.redb --format nquads -o backup.nq
tm convert memory.redb memory.db --backend sqlite
tm compact memory.redb
tm bench --scale 100000 --reps 5                # SQLite against redb on this machine
tm reason memory.redb --rules socrates.n3      # what follows, written nowhere
tm rules put memory.redb urn:tiramemsu:v:mortality socrates.n3
tm materialize memory.redb urn:tiramemsu:v:mortality --dry-run
tm datalog memory.redb reporting.dl 'reports(v:alice, X)'   # rows with their valid time
tm program put memory.redb urn:tiramemsu:v:reporting policies.dl
tm check memory.redb urn:tiramemsu:v:reporting              # constraint violations
tm program enforce memory.redb urn:tiramemsu:v:reporting     # refuse writes that violate its error constraints
```

| Command | What it does |
|---|---|
| `create <file> [--backend sqlite\|redb] [--durability normal\|immediate\|none]` | Creates a new database; an existing file is refused (`AlreadyExists`). |
| `info <file> [--json]` | Backend, format version, live and total statements, dictionary terms, transactions, file size. |
| `query <file> --sparql\|--cypher <text\|@file>` | Runs a query. `--format table\|json\|csv\|tsv\|ntriples`; views `--as-of <tx>`, `--as-of-instant <ms>`, `--history`, `--valid-at <ms>`; `--timeout <ms>`; `--explain` prints the plan (SPARQL). |
| `import <file> <rdf>...` | Loads RDF files in bulk-import transactions of `--chunk` statements (100 000). Blank-node labels are scoped to their file. |
| `export <file> [--format nquads\|ntriples] [-o out]` | Writes the live statements, and for N-Quads each named graph membership. |
| `convert <src> <dst> --backend <b>` | Lossless conversion, replaying the whole history (`tiramemsu::convert`). |
| `compact <file>` | Reclaims free space in a redb file. SQLite files have nothing to compact. |
| `bench [--scale N] [--reps R] [--out results.json]` | Runs the reduced backend benchmark (SQLite and redb) and prints the report. The comparison with Oxigraph is in `bench/backends`. |
| `reason <file> --rules <path>\|--rule-set <iri>` | Runs N3 rules and prints one conclusion per line with its valid time; writes nothing. `--as-of <tx>`, `--as-of-instant <t>`, `--valid-at <t>` or `--valid-during <from>..<to>`, `--graph <iri>` (repeatable), `--atemporal`, `--max-segments`, `--proof`, `--json`. |
| `rules put <file> <iri> <path>`, `rules show <file> <iri> [--json]` | Stores a rule set (a new version supersedes the old one) and prints it. |
| `materialize <file> <iri>` | Writes the conclusions of a stored rule set as derived beliefs. `--valid-during`, `--graph`, `--require-complete`, `--dry-run` (lists what would be derived, writes nothing), `--json`. |
| `datalog <file> <program> [goal]` | Runs a Datalog program (a file, or the IRI of a stored program) and prints one row per line, `X=v:bob  always`, with the row's valid-time intervals; writes nothing. Without a goal it runs the program's first `?-` goal. `--as-of <tx>`, `--as-of-instant <t>`, `--valid-at <t>` or `--valid-during <from>..<to>`, `--graph <iri>` (repeatable), `--atemporal`, `--require-complete`, `--no-demand`, `--proof`, `--explain` (prints the plan), `--json`. |
| `check <file> <program>` | Evaluates the program's constraints and prints each violated one with its severity and violations (bindings, intervals, statement ids). `--only <name>` (repeatable), the view and run options of `datalog`, `--json`. Violations are a result: the exit status is 0. |
| `program put <file> <iri> <path>`, `program show <file> <iri> [--json]` | Stores a Datalog program (a new version supersedes the old one) and prints it with its rule, goal, constraint and stratum counts and whether it is enforced (`enforced` in `--json`). |
| `materialize-program <file> <iri>` | Writes the conclusions of a stored Datalog program as derived beliefs (a file of text is refused: the program must be stored with `program put`). `--valid-during`, `--graph`, `--require-complete`, `--dry-run` (lists what would be derived, writes nothing), `--json`. Lists derived, existing, widened and reconciled statements; the summary goes to stderr. |
| `program enforce <file> <iri>`, `program unenforce <file> <iri>` | Turns enforcement of a stored program's `error` constraints on or off (`(iri dl:enforced true)`). Enforcing checks the whole store first: data that already violates a constraint fails with `ConstraintViolated` and the violations on stderr, and a constraint the commit-time check cannot evaluate with `ConstraintNotEnforceable`. Once enforced, any write (`import`, `materialize`, ...) that would introduce a violation is refused with `ConstraintViolated` and commits nothing. |

Instants for the reasoning commands are epoch milliseconds or RFC 3339 dates and date-times; a window side may be left empty (`2020-01-01..`). The N3 commands come with the cargo feature `n3` and the Datalog commands with the feature `datalog`, both on by default.

Errors print their typed name and message on stderr (`Parse: …`, `BackendMismatch: …`) and exit with code 1. Usage errors exit with code 2.

Licensed under MIT or Apache-2.0, at your option.
