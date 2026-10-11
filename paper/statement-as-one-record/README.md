# When Is a Temporal Statement Store a Statement Graph?

**Revision 3, 11 October 2026.** *Export Boundaries for Occurrence Records with Two Clocks* (revisions 1–2 were titled *The Statement as One Record*). Volodymyr Pavlyshyn, independent researcher.

- [PDF](statement-as-one-record.pdf)
- [LaTeX source](statement-as-one-record.tex) and [bibliography](references.bib)
- [Revision notes](REVISION_NOTES.md) and [source audit](SOURCE_AUDIT.md)
- [Revision 1 referee report](REVIEW.md) and [original review brief](REVIEW_BRIEF.md)
- [Executable validation](validate.py), [results](validation-results.json), [generated manuscript counts](validation-counts.tex) and [engine trace](engine-trace.json)

The paper characterizes identity-preserving export of a temporal statement store into a Statement Graph. Closure and acyclicity are necessary and sufficient. The export kernel, the least fixpoint of `F(X) = {r : refs(r) ⊆ X}`, is the unique greatest exportable subset; a linear topological peel computes it with an export order, and one bit per record maintains it incrementally. The dual reference closure keeps lineage the kernel drops. Under immutable, never-deleted records the full history is closed and its cycles lie inside single replay batches, so exporting the history loses only correction cycles. Time is encoded as reserved statements for a lossless roundtrip into the RDF-star branch of the OneGraph image.

## Evidence and limits

- Hand proofs; no mechanization.
- Validation: 217,625 labeled stores of up to three records, 72,567 four-record reference structures up to relabeling, 20,000 random stores of 4–7 records, 1,500 random protocol histories (66,000 snapshot checks), 3,000 reference-closure minimality checks, focused cases and image mutations. Three kernel implementations are compared with an exhaustive subset oracle.
- Engine evidence: `crates/tm-core/tests/export_boundary.rs` runs both running-example corrections on the engine; with `TM_EXPORT_TRACE` set it writes `engine-trace.json`, which the validator recomputes independently. The kernel is not an engine API.
- No LPG image, RDF 1.2 interchange or query-equivalence claim. The SQLite timings are inherited, not rerun, and are in an appendix.

## Validate and build

Python 3 (standard library), Rust, and a LaTeX installation with `pdflatex` and `bibtex` are required.

```sh
TM_EXPORT_TRACE=$PWD/paper/statement-as-one-record cargo test -p tm-core --test export_boundary
cd paper/statement-as-one-record
python3 validate.py
python3 build.py
```

Validation writes deterministic result/count files with hashes of the script and engine trace. The build refuses stale validation, runs TeX and BibTeX in a temporary directory, and rejects undefined references/citations and overfull boxes before replacing the PDF.
