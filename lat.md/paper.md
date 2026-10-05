# Paper Artifacts

The layered-bitemporal-graphs manuscript is a research preprint with retained verification evidence and a separately prepared arXiv source upload.

## arXiv Package

The package builder produces a source-only ZIP, metadata and a preview PDF, and checks compilation from the ZIP in isolation.

[[paper/artifact/package_arxiv.py#main]] copies the current manuscript and bibliography, generates a matching `.bbl`, then creates a ZIP containing only `.tex`, `.bbl` and `.bib` files. Diagrams are inline TikZ. Instructions, metadata, hashes and preview PDF remain outside the upload archive.

Verification extracts the exact ZIP into a fresh directory, compiles with the included `.bbl` without rerunning BibTeX there, compares PDF text with a full source build, and checks citations, links and warnings. New packages use fresh output directories so previous deliverables are preserved.

Submission through the author's arXiv account and server-side compilation are separate steps. The source ZIP does not publish the implementation artifact or create a permanent release DOI.

The revision-4 submission folder is kept to four files: upload ZIP, metadata text, instructions and preview PDF. Duplicate metadata, a separate abstract and the original verification report are retained in `paper/review-v4/arxiv-package-details/`.

## Zenodo Deposit

`paper/zenodo/` holds Zenodo-ready metadata (`metadata.json`) and upload steps for the revision-4 PDF. Nothing is uploaded from the repository; publishing and the resulting DOI are manual steps by the author.

## Manuscript

`paper/layered-bitemporal-graphs.tex` and `references.bib` are the source; the README summarises the current revision.

The original `layered-bitemporal-graphs.pdf` is the revision-1 build. The revision-4 build is the arXiv preview PDF, which the website serves. Review reports and earlier revision folders are kept outside the repository, so the paper's notes name them without linking.

Revision 4 describes conformance repairs (live statement endpoints, closed retention on correction, transaction-date guards) that shipped in 0.4.0, with the date guards as storage format 4. The paper README, the paper page and the companion article say which release has them.

## Metagraph Encodings Paper

`paper/metagraphs-from-identified-triples/` holds the second, mostly static paper: invertible encodings of metagraphs over identified triples, with its checkers, validators and a Lean proof.

The paper's thesis is that statement identity is cheap and invertibility is a separate integrity obligation. It states exact image constraints for directed, higher-order, extensional, recursive-member and attributed families, and shows each constraint is necessary with a witness that violates only it. The design it formalises is [[recipes#Metagraphs]] and [[data-model#Named Graphs#Statement Graphs]]. Row fragments are shape classes, not normal forms. The engine does not enforce the images, and the folder README lists the evidence limits.

### Snapshot Policies and Versions

The temporal theorem has two policies: containment of valid time, or asserted rows plus retained referents filtered by transaction time alone.

Under the second policy an annotation may be valid outside its bearer's interval, and containment is the special case. Extensional images are fragile under time because extensionality is a property of each snapshot of a history. Identity across versions uses supersession lineage rows with chain admissibility, and propagating a revision to dependents stays the cascade-and-replay of the first paper.

### Executable Metapaths and Rank Consumers

Firing closure decides in linear time whether an edge set can fire under all-inputs-required semantics, and rank prices loading and retraction.

A walk metapath is executable when the dependency graph is acyclic or every edge has one input, and the cyclic case is the counterexample. Reference rank gives the minimum number of referentially complete load batches, one more than the maximum rank, and bounds retraction-cascade rounds by maximum rank minus the retracted row's rank. Anchored encodings keep structural rows at rank one, compact binary ones make rank equal endpoint depth. These are pass counts over the reference graph, not performance claims.

### Paper Validation

Evidence is exhaustive bounded enumeration, seeded randomized tests, SHACL and SPARQL validators, and a Lean proof, and it is all finite or partial.

`check_encodings.py` and `check_extensions.py` are deterministic and write the paper's two tables. The Lean development proves the directed, higher-order and extensional roundtrip under canonical naming only. The validators under `validators/` express the image constraints over a projected row view and are tested for agreement with the Python validators; they do not make the engine enforce anything.

## Website Page

`site/paper/index.html` gives the title, abstract, the four main results, status, the engine caveat and a BibTeX entry, and links the PDF, the Zenodo DOI (10.5281/zenodo.23139228), the LaTeX source and the companion article. See [[architecture#Project Website]].
