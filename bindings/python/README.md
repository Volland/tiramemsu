<p align="center"><img src="https://cdn.jsdelivr.net/gh/Volland/tiramemsu@main/site/assets/logo.svg" width="140" alt="Tiramemsu: a brain taking a bite out of a tiramisu whose layers are a graph"></p>

<h1 align="center">tiramemsu</h1>

<p align="center"><em>Your agent's brain loves Tiramemsu.</em><br>
Layered, never-forget memory for agents, for Python.</p>

<p align="center"><a href="https://volland.github.io/tiramemsu/">Website</a> · <a href="https://volland.github.io/tiramemsu/articles/time-travel.html">Time travel explained</a> · <a href="https://volland.github.io/tiramemsu/articles/layered-graphs.html">Layered graphs explained</a> · <a href="https://github.com/Volland/tiramemsu">GitHub</a></p>

Tiramemsu is an embedded graph database on SQLite. Every fact has its own id, so a fact can carry a confidence, a source or a belief about it, layer on layer. Every change is kept, with when the database learned it and when it was true in the world. This package is the Python binding: a native module (PyO3, stable ABI) and a typed, synchronous API.

> **Status: new.** Tiramemsu was designed and implemented in September 2026. This package has not been published to PyPI yet, so the install line below works after the first release. Until then, build it from source.

## Install

```sh
pip install tiramemsu
```

Python 3.9 or later. Each platform gets one prebuilt wheel (Linux x86_64 and aarch64, musllinux x86_64, macOS x86_64 and arm64, Windows x64), and SQLite is compiled in, so nothing else is needed. The package is typed (`py.typed`) and checked with `mypy --strict`.

<details>
<summary>Build from source instead</summary>

You need Python 3.9 or later and the Rust toolchain pinned by the repository (1.91.1).

```sh
git clone https://github.com/Volland/tiramemsu
cd tiramemsu/bindings/python
python3 -m venv .venv && source .venv/bin/activate
pip install maturin pytest mypy
maturin develop        # or: maturin develop --release
pytest
```

If another `rustc` is first on your `PATH` (for example Homebrew's), put rustup's first: `export PATH="$HOME/.cargo/bin:$PATH"`.

</details>

## Quick start

Alice joins Acme in 2020, with a confidence of 0.8. We later learn she left at the end of 2023, and then that she joined Globex in March 2024.

```python
from tiramemsu import Database, Iri

V = "urn:tiramemsu:v:"  # written `v:` in queries
alice, works_at, acme, globex, confidence = (
    Iri(V + name) for name in ("alice", "worksAt", "acme", "globex", "confidence")
)

db = Database("memory.db")

# 1. A fact is a statement with its own id, so it can carry a layer.
with db.transact() as tx:
    job = tx.assert_(alice, works_at, acme, valid_from="2020-01-01")
    tx.assert_(job, confidence, 0.8)  # `job` is a Ref: the statement, before it has an id
first = tx.report.asserted[0]         # its eid, known once the transaction has committed

# 2. Correct it. The layer moves to the new statement; nothing is deleted.
with db.transact() as tx:
    tx.supersede(first, valid_to="2024-01-01")

# 3. A new fact.
with db.transact() as tx:
    tx.assert_(alice, works_at, globex, valid_from="2024-03-01")

# 4. Ask the same question at different times.
q = "SELECT ?org WHERE { v:alice v:worksAt ?org }"

def orgs(view):
    return [row["org"].value.rsplit(":", 1)[-1] for row in view.sparql(q)]

orgs(db.now())                                  # ['acme', 'globex']  both episodes are live
orgs(db.as_of(tx=1))                            # ['acme']            what we believed after tx 1
orgs(db.as_of(tx=1).valid_at("2026-01-01"))     # ['acme']            we thought she was still there
orgs(db.now().valid_at("2026-01-01"))           # ['globex']          what is true today
orgs(db.now().valid_at("2024-02-01"))           # []                  between the two jobs
```

The same store speaks Cypher, and a layer is a relationship property:

```python
db.now().cypher("MATCH (a)-[r:worksAt]->(c) RETURN c, r.confidence AS conf")
# columns ['c', 'conf'], rows [[acme, 0.8], [globex, None]]  (Globex has no confidence layer)
```

## Two clocks

A view chooses when you look and when the fact was true. Every read takes a view, and views are immutable.

| You want | Code |
|---|---|
| What we believe now | `db.now()` |
| What we believed after transaction `n` | `db.as_of(tx=n)` |
| What we believed at a wall-clock time | `db.as_of(instant="2026-09-01T12:00:00Z")` |
| What was true on a date | `view.valid_at("2026-01-01")` |
| Every statement ever, with retracted ones | `db.history()` |

`valid_at` combines with any of them. Valid time is off unless you ask for it. [Time travel and bitemporality, explained](https://volland.github.io/tiramemsu/articles/time-travel.html) has the full story.

## Writing

`db.transact()` is a context manager. It records the operations of the block and commits them as one transaction when the block exits cleanly, and the report is on `tx.report`. If the block raises, nothing is committed. `db.transact(ops)` takes a list of operation dicts instead. `assert_`, `create` and `supersede` return a `Ref` that later calls in the same block accept as a statement id, a subject or an object.

| Method | What it does |
|---|---|
| `assert_(s, p, o, valid_from=, valid_to=, on_existing=)` | Adds a statement; idempotent when a live one with an overlapping valid time exists |
| `create(s, p, o, ...)` | Always adds one, for parallel edges |
| `retract(eid)` | Ends belief in a statement and every layer about it |
| `retract_matching(s=, p=, o=)` | Retracts every live match |
| `supersede(eid, o=, valid_from=, valid_to=)` | Corrects a statement and replays its layers on the new one. A bound you leave out is kept; `None` clears it |
| `confirm(eid)` | Records that another source agrees |
| `meta(p, o)`, `upsert(p, o)`, `new_node()` | Transaction metadata, unique upsert, an anonymous node |
| `add_to_graph(eid, g)`, `remove_from_graph`, `clear_graph`, `create_graph`, `drop_graph` | Named graphs as tags on statements |
| `cypher(text, params=None)` | A Cypher query that may write, inside the same transaction |

Times accept a `datetime`, a `date`, epoch milliseconds or an RFC 3339 string. `db.transact(dry_run=True)` reports what would happen and commits nothing. `db.speculate(ops, queries)` applies changes hypothetically, runs queries on the result, and keeps nothing.

## Reading

| Method | Returns |
|---|---|
| `view.sparql(text)` | A select result (iterate it for rows of variable to term; `.vars` lists the variables), an ask, a graph, or an update result |
| `view.cypher(text, params=None)` | A result with `columns` and `rows` |
| `view.triples(s=, p=, o=)` | `Statement` values with `eid`, `s`, `p`, `o`, `t_add`, `t_ret`, `valid_from`, `valid_to`, `ret_kind` |
| `view.path(start, expr, mode="reach", max_hops=None)` | Endpoints with hop counts. `mode` is `"reach"`, `"trail"`, `"anyShortest"` or `"allShortest"` |
| `view.events(since=0)` | The change log after a transaction |
| `view.graphs()`, `view.graph_members(g)`, `view.values(s, key)` | Named graphs and values |

`expr` is SPARQL property-path syntax with the predeclared prefixes `v:`, `sys:`, `tm:`, `rdf:` and `xsd:`, for example `v:knows+`. SPARQL supports RDF 1.2 annotations: `{| v:confidence ?c |}` reads a layer.

## Terms

| Python | Stored as |
|---|---|
| `str`, `bool` | plain string, boolean |
| `int` | `xsd:integer`, exact at any size |
| `float` | `xsd:double` |
| `datetime`, `date` | `xsd:dateTime`, `xsd:date`; they come back as the same Python types. A naive `datetime` is taken as UTC |
| `Iri(s)` | IRI |
| `Literal(lex, datatype=)`, `Literal(lex, lang=)` | typed or language-tagged literal |
| `Node(n)`, `BNode(n)`, `Stmt(eid)`, `TxId(n)` | anonymous node, blank node, statement id, transaction |

## Errors

Every failure raises `TiramemsuError` with a `.code`:

| Code | Meaning |
|---|---|
| `Parse` | SPARQL, Cypher or path text is invalid |
| `InvalidArgument` | The call itself is malformed: an unknown operation, a missing field, a term of the wrong shape |
| `NotLive` | The statement does not exist or was already retracted |
| `Unsupported` | Valid but outside the supported subset, such as a Cypher write through a read view |
| `UniqueViolation` | A `sys:unique` predicate already has a live holder of that value |
| `ValueTypeMismatch` | The object does not match the predicate's `sys:valueType` |
| `CascadeLimitExceeded` | A retraction would touch more than `max_cascade` statements |
| `PathLimitExceeded` | A path search exceeded its state limit |
| `InvalidPatch` | A `supersede` patch tried to change the subject or predicate |
| `Sqlite` | A SQLite failure, such as a locked or unreadable file |

## Options

`Database(path, readers=, busy_timeout_ms=, term_cache_capacity=, optimize_every=, path_max_hops=, path_max_states=)`. It works as a context manager, and Python threads can query one `Database` in parallel, because the GIL is released during each call.

## Good to know

- The API is synchronous, like the Rust core. Reads run in parallel on a small connection pool, and writes serialize on one writer.
- Nothing is ever deleted: the SQLite file rejects `DELETE`. Forgetting means retracting, and the past stays exact.
- Results cross the native boundary as JSON, which is fine for agent memory and not meant for million-row result sets.
- There is no MCP server, WASM build or network server yet.

## License

MIT OR Apache-2.0, at your option. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
