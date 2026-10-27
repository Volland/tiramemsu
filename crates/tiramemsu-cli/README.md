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

Errors print their typed name and message on stderr (`Parse: …`, `BackendMismatch: …`) and exit with code 1. Usage errors exit with code 2.

Licensed under MIT or Apache-2.0, at your option.
