# Statement ids versus RDF 1.2 reifiers

`bench.py` compares tiramemsu's statement ids with a store that has quads plus RDF 1.2 reifiers (oxilite's model) on the same 750 000 triples, 10 % of whose edges are annotated. It is the evidence behind D27 (`lat.md/storage.md#Multi-Eid Predicates`).

```sh
python3 bench.py
```

It needs only the standard library, and its database files go to a temporary directory.
