# Paper Artifacts

The layered-bitemporal-graphs manuscript is a research preprint with retained verification evidence and a separately prepared arXiv source upload.

## arXiv Package

The package builder produces a source-only ZIP, metadata and a preview PDF, and checks compilation from the ZIP in isolation.

[[paper/artifact/package_arxiv.py#main]] copies the current manuscript and bibliography, generates a matching `.bbl`, then creates a ZIP containing only `.tex`, `.bbl` and `.bib` files. Diagrams are inline TikZ. Instructions, metadata, hashes and preview PDF remain outside the upload archive.

Verification extracts the exact ZIP into a fresh directory, compiles with the included `.bbl` without rerunning BibTeX there, compares PDF text with a full source build, and checks citations, links and warnings. New packages use fresh output directories so previous deliverables are preserved.

Submission through the author's arXiv account and server-side compilation are separate steps. The source ZIP does not publish the implementation artifact or create a permanent release DOI.

The revision-4 submission folder is kept to four files: upload ZIP, metadata text, instructions and preview PDF. Duplicate metadata, a separate abstract and the original verification report are retained in `paper/review-v4/arxiv-package-details/`.

## Manuscript

`paper/layered-bitemporal-graphs.tex` and `references.bib` are the source; `REVISION-*.md` record what each revision changed.

The original `layered-bitemporal-graphs.pdf` is the revision-1 build. The revision-4 build is the arXiv preview PDF, which the website serves. Review reports and earlier revision folders are kept outside the repository, so the paper's notes name them without linking.

Revision 4 describes conformance repairs (live statement endpoints, closed retention on correction, transaction-date guards) that the released engine 0.3.0 does not contain. The paper README, the paper page and the companion article state this, and must be updated when the repairs ship.

## Website Page

`site/paper/index.html` gives the title, abstract, the four main results, status, the engine caveat and a BibTeX entry, and links the PDF, the LaTeX source and the companion article. See [[architecture#Project Website]].
