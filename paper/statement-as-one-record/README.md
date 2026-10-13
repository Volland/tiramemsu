# When Is a Temporal Statement Store a Statement Graph?

**Revision 4, 12 October 2026.** *Export Boundaries for Occurrence Records with Two Clocks* (revisions 1–2 were titled *The Statement as One Record*). Volodymyr Pavlyshyn, independent researcher.

- Zenodo: [10.5281/zenodo.23335652](https://doi.org/10.5281/zenodo.23335652) (concept DOI, all versions; PDF and validation archive); website page `site/paper/export-boundaries.html`.
- [PDF](statement-as-one-record.pdf)
- [LaTeX source](statement-as-one-record.tex) and [bibliography](references.bib)
- [Revision notes](REVISION_NOTES.md) and [source audit](SOURCE_AUDIT.md)
- [Revision 1 referee report](REVIEW.md); revision 4 answers a meta-review summarized in the revision notes and [original review brief](REVIEW_BRIEF.md)
- [Executable validation](validate.py), [results](validation-results.json), [generated manuscript counts](validation-counts.tex) and [engine trace](engine-trace.json)
- [Lean mechanization](lean/README.md) of the combinatorial results
- [Zenodo metadata, upload steps and validation-archive builder](zenodo/README.md)

The paper characterizes identity-preserving export of a temporal statement store into a Statement Graph. Closure and acyclicity are necessary and sufficient. The export kernel, the least fixpoint of `F(X) = {r : refs(r) ⊆ X}`, is the unique greatest exportable subset; a linear topological peel computes it with an export order, and one bit per record maintains it incrementally. The dual reference closure keeps lineage the kernel drops. Under immutable, never-deleted records the full history is closed and its cycles lie inside single replay batches, so exporting the history loses only correction cycles. Time is encoded as reserved statements for a lossless roundtrip into the RDF-star branch of the OneGraph image, and conjunctive queries over the asserted part are answered as in the store, up to a quoting bijection, when contents are distinct. A batch-local acyclic-replay rule makes the whole history exportable. RDF 1.2 reifiers export every store, cycles included, but assert per triple; the property-graph branch admits only edges and unreferenced properties.

## Evidence and limits

- The PDF's proofs are by hand. `lean/` re-proves the criterion, the kernel and obstructions, the reference closure, snapshot and history closure, the acyclic-replay rule, the exportable bit, the temporal roundtrip, the query view and the reifier roundtrip in Lean 4 (core only, no `sorry`). The LPG proposition and the cost bounds are not mechanized. The PDF text predates this and still says "no mechanization".
- Validation: 217,625 labeled stores of up to three records, 72,567 four-record reference structures up to relabeling, 20,000 random stores of 4–7 records, 1,500 random protocol histories (66,000 snapshot checks), 1,500 histories under the acyclic-replay rule (337 supersessions rejected), 3,000 reference-closure minimality checks, focused cases and image mutations. Every store also checks the query view, the RDF 1.2 reifier roundtrip and the LPG direct-export condition. Three kernel implementations are compared with an exhaustive subset oracle.
- Engine evidence: `crates/tm-core/tests/export_boundary.rs` runs both running-example corrections on the engine; with `TM_EXPORT_TRACE` set it writes `engine-trace.json`, which the validator recomputes independently. The kernel is not an engine API.
- LPG: direct-export condition only (no temporal roundtrip, no greatest subset). RDF 1.2: reifier encoding, not full semantics. Queries: conjunctive patterns and equality-based SPARQL operators under set semantics; no time travel, aggregates or Cypher.
- No performance claims; the inherited SQLite timings of revisions 2–3 were removed.

## Validate and build

Python 3 (standard library), Rust, and a LaTeX installation with `pdflatex` and `bibtex` are required.

```sh
TM_EXPORT_TRACE=$PWD/paper/statement-as-one-record cargo test -p tm-core --test export_boundary
cd paper/statement-as-one-record
python3 validate.py
python3 build.py
(cd lean && lake build)
python3 zenodo/build_archive.py   # validation archive for Zenodo
```

Validation writes deterministic result/count files with hashes of the script and engine trace. The build refuses stale validation, runs TeX and BibTeX in a temporary directory, and rejects undefined references/citations and overfull boxes before replacing the PDF.
