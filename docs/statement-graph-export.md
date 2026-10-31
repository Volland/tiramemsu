# Statement-graph export: a guide

`tm export --format sg-nt12` writes a memory whole: every statement with its id, its transaction lifetime and its valid interval, as RDF 1.2. The plain export (`--format ntriples` or `nquads`) writes only what is live now, without ids or clocks. This one keeps all of it, in a format that [LeanRDF](https://github.com/Volland/rdf-lean) defines and proves exact in Lean. Design notes are in `lat.md/api.md` (*Statement Graph Export*, decision D53).

## Five minutes

```sh
tm export memory.db --format sg-nt12                          # assert what is live now
tm export memory.db --format sg-nt12 --as-of 3                # ... what was live after transaction 3
tm export memory.db --format sg-nt12 --history                # ... every statement
tm export memory.db --format sg-nt12 --valid-at 1800000000000 # ... only what is valid at that instant
```

`--valid-at` combines with any of the other three, as in queries. The flags are refused with the other formats.

## What a statement becomes

Take a route, and a distance about that route that was wrong and corrected in transaction 4:

```rust
let fra_lhr = tx.assert(ex("FRA"), ex("route"), ex("LHR"), Valid::ALWAYS)?.eid();   // tx 1, eid 1
let wrong = tx.assert(fra_lhr, ex("distance"), Value::Int(665), Valid::ALWAYS)?.eid(); // tx 2, eid 2
// tx 4
let fixed = tx.supersede(wrong, Patch::object(Value::Int(655)))?;                     // eid 4
tx.assert(fixed, ex("source"), ex("airlineDB"), Valid::ALWAYS)?;                       // eid 6
```

Each statement `e` becomes:

| Triple | When |
|---|---|
| `<urn:tiramemsu:stmt:e> rdf:reifies <<( s p o )>>` | always |
| `stmt:e tm:txAdded N` | always |
| `stmt:e tm:txRetracted N` | when retracted |
| `stmt:e tm:validFromMs MS`, `stmt:e tm:validToMs MS` | when the statement has them |
| `s p o` | when the statement is live in the view |

A statement inside `s` or `o` is written `<urn:tiramemsu:stmt:n>`, the same IRI SPARQL results use. All clocks are `xsd:integer`, and `tm:` is `urn:tiramemsu:tm:`. In the `now` view the corrected memory gives, among other lines:

```
<urn:tiramemsu:stmt:1> <http://example.org/distance> "655"^^<http://www.w3.org/2001/XMLSchema#integer> .
<urn:tiramemsu:stmt:2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#reifies> <<( <urn:tiramemsu:stmt:1> <http://example.org/distance> "665"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .
<urn:tiramemsu:stmt:2> <urn:tiramemsu:tm:txRetracted> "4"^^<http://www.w3.org/2001/XMLSchema#integer> .
```

The wrong distance is still there as a reifier with its retraction. Only the corrected one is asserted. `--as-of 3` asserts 665 instead, and `--history` asserts both. The full outputs for all four views are in `crates/tiramemsu-cli/tests/sg/correction.*.nt`.

## What is left out, and what is refused

- **Left out:**
  - `urn:tiramemsu:sys:` statements, which are engine bookkeeping, as in the plain export. The supersede above writes one, which is why eid 5 is missing. A statement about a left-out statement still exports, and its reference points at an id that is not in the file.
  - Statements added and retracted in the same transaction. No view ever shows them.
- **Refused:** content that uses a `urn:tiramemsu:stmt:` IRI as a plain IRI, or `rdf:reifies` as a predicate. On reading back, either would mean something else.

## Why `tm:validFromMs` and not `tm:validFrom`

In SPARQL and Cypher, `tm:validFrom` and `tm:validTo` are virtual predicates whose values are `xsd:dateTime`. The export writes epoch milliseconds, the form LeanRDF's decoder is proven on, so it uses names of its own. No IRI means two things.

## Checking an export with LeanRDF

LeanRDF's `leanrdf-statements` package defines this format and proves its decoder exact:
- `decode_toRDF` says that decoding an export gives back the statements;
- `toRDF_decode` says that a file which decodes is exactly the export of a well-formed set of statements.

A stray triple, a missing clock or a missing assertion is an error that names the statement:

```sh
tm export memory.db --format sg-nt12 --history > memory.nt
cd rdf-lean/statements
lake exe leanrdf-sg decode ../memory.nt --history   # exit 0 and JSON-lines records, or exit 2 and the reason
lake exe leanrdf-sg kernel records.jsonl            # the part with a Statement Graph export (the paper's kernel)
```

The output is canonical RDF 1.2 N-Triples, in LeanRDF's triple order and with its string escapes, so `leanrdf-sg export` of the decoded records reproduces the file byte for byte. LeanRDF keeps a copy of `crates/tiramemsu-cli/tests/sg/` and checks this on every CI run. `TM_SG_UPDATE=1 cargo test -p tiramemsu-cli --test sg` re-records the files after an intended change; LeanRDF's `scripts/statements-tm-record.sh` then copies them again.

## Not supported

- Reading the format back: `tm import` refuses triple terms, and the clocks would not become intervals.
- Named graphs: membership is not part of the format.
- "Statement graph" here is the paper's sense, a Gelling–Fletcher–Schmidt Statement Graph (see *When Is a Temporal Statement Store a Statement Graph?*, Zenodo 10.5281/zenodo.23335652). It is not the named graph named by a statement.
