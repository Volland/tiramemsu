# Metagraph Encodings over Identified Triples

**Exact Images, Temporal Integrity, and Structural Depth.** Revision 2, 5 October 2026. Author: Volodymyr Pavlyshyn.

- [PDF](metagraphs-layered-graphs.pdf) and [LaTeX source](metagraphs-layered-graphs.tex): revised paper (19 pages).
- [Checker](check_encodings.py) and [extension checks](check_extensions.py), [results](validation-results.json), `validation-counts.tex` and `ablation-table.tex`: reproducible validation and the two generated tables.
- [validators/](validators/): SHACL shapes and SPARQL queries for the image constraints, with a test of agreement against the Python validators.
- [lean/](lean/): Lean 4 proof of the directed, higher-order and extensional roundtrip under canonical naming.
- [Revision notes](REVISION_NOTES.md) and [source audit](SOURCE_AUDIT.md): what changed in response to review, sources checked, remaining limits.
- [Build script](build.py): builds the PDF in a temporary directory.
- `references.bib`: only the references cited by the manuscript.

## Reproduce

From this directory, with Python 3.10+ and a TeX distribution:

```sh
python3 check_encodings.py --output validation-results.json   # also writes the two .tex tables
python3 build.py
```

The checkers use only the Python standard library and are deterministic (the randomized tests use a fixed seed and are independent of `PYTHONHASHSEED`). They exit unsuccessfully on a failed assertion; JSON and TeX are written only after every check passes. Counts refer to models, stores or query configurations, not numbers of proofs; the JSON records which sets are exhaustive and which are sampled.

The SHACL/SPARQL validators need `rdflib` and `pyshacl` (see `validators/README.md`). The Lean proof needs the Lean toolchain: `cd lean && lake build`.

The existing engine recipe can be checked from the repository root:

```sh
cargo test -p tiramemsu --test recipes metagraph_containers_nesting_and_fold --offline
```

## Scope

The paper presents hand proofs with finite, sampled and partially mechanized support. Its claim is that identified-triple encodings of explicitly specified finite metagraph structures are invertible only on exact images, and that each image constraint is necessary. Further results: two snapshot policies (asserted rows, or asserted rows plus retained referents), a supersession relation for identity across versions, linear-time executable-metapath recognition and its relation to walk reachability, and bulk-load strata and cascade depth as consumers of reference rank.

Not claimed: engine enforcement of the images, a retrieval or performance evaluation, a normalization procedure (the row fragments are shape classes, not normal forms), a mechanized proof beyond the directed family under canonical naming, or priority over the three recent metagraph works compared at abstract level. Revision-1 limits that remain are listed in the paper's Discussion and in `SOURCE_AUDIT.md`.
