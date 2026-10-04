# Paper: Layered Bitemporal Graphs

**Revision 4, 3 October 2026.** *Statement Identity, Two Clocks, and Metagraphs as Tags.* An open-access preprint draft (CC BY 4.0) describing an integrity calculus for statement occurrences and temporal metagraph snapshots.

- Zenodo archive: [10.5281/zenodo.23139228](https://doi.org/10.5281/zenodo.23139228)
- [Revision 4 PDF](arxiv/revision-4/layered-bitemporal-graphs-arxiv-preview-v4.pdf)
- [LaTeX source](layered-bitemporal-graphs.tex) and [bibliography](references.bib)
- Post-revision review and reference audit
- [Retained verification artifact](artifact/README.md)
- Original review and preserved revision 1

The original `layered-bitemporal-graphs.pdf` remains the revision-1 build. The revision-4 PDF above is the arXiv preview build, also served on the website at `site/paper/layered-bitemporal-graphs-v4.pdf`. Review reports and earlier revision folders are kept outside the repository. This keeps existing PDF files intact; fresh builds publish to a new filename recorded in `review-v4/build-check.json`.

Revision 2 remains available in revisions/v2, with its review.

**Engine status.** Revision 4 describes conformance repairs (live statement endpoints, closed retention on correction, transaction-date guards in a storage format) that shipped in tiramemsu 0.4.0. The date guards are storage format 4 (format 3 holds the saved-answer tables), and a file migrated to format 4 no longer opens in 0.3.0. Version 0.3.0 and earlier still have the gaps that revision 3 reported.

## Build and verification

Use the retained build script from the repository root:

```sh
python3 paper/artifact/build.py --revision=4 --publish
```

It needs `pdflatex`, `bibtex`, `pdftotext`, and Python with PyMuPDF for PDF checks. The LaTeX packages are `fontenc`, `inputenc`, `lmodern`, `geometry`, `amsmath`, `amssymb`, `amsthm`, `mathtools`, `microtype`, `booktabs`, `array`, `xcolor`, `url`, `natbib`, `caption`, `tikz`, and `hyperref`. Install any missing packages through your TeX distribution.

The underlying sequence is `pdflatex`, `bibtex`, then repeated `pdflatex` until citation and cross-reference warnings disappear. The script uses a fresh temporary build directory and checks all rendered arXiv identifiers and DOI links.

## Guarantees and limits

The core calculus assumes a referentially complete history. Structural closure exempts declared lineage references from liveness, while still requiring their targets to exist. Correction preserves non-root references under explicit admissibility conditions; an object patch can change root references. Metagraph snapshot lifting assumes normal form and temporal well-formedness. Compact LR realisability also needs acyclic dependencies; realisability excludes empty component lifetimes.

Retained engine probes now verify prevention of the four reported conformance gaps. Format 3 adds transaction-date guards, ordinary writes reject missing/retracted statement endpoints, and correction prunes structural layers of dropped memberships before allocation. Existing databases migrate atomically without rewriting historical rows; previously invalid states are not automatically sanitized. Raw SQL remains subject to the documented protocol and intact schema/metadata assumptions.

The artifact supplies correctness evidence, not a performance benchmark or evidence of agent-memory quality. Permanent artifact archiving and any retained systems performance claims need additional work before submission.

Revision 3 is preserved in revisions/v3, with its review.

## arXiv upload package

The [tested source ZIP](arxiv/revision-4/layered-bitemporal-graphs-arxiv-v4.zip), [submission metadata](arxiv/revision-4/submission-metadata.txt), [preview PDF](arxiv/revision-4/layered-bitemporal-graphs-arxiv-preview-v4.pdf) and [upload instructions](arxiv/revision-4/README.md) are prepared locally. The package contains only LaTeX source, the matching `.bbl`, and `references.bib`. Its isolated compilation and checks are recorded in `review-v4/arxiv-package-details/package-check.json`.

Regenerate into a fresh output directory with `python3 paper/artifact/package_arxiv.py --output /tmp/tiramemsu-arxiv-new`. The default output is `paper/arxiv/revision-4/` and existing directories are preserved. This preparation does not upload or submit the paper to arXiv.
