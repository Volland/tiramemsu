# arXiv upload package — internal manuscript revision 4

Upload **layered-bitemporal-graphs-arxiv-v4.zip** through your arXiv author account. The archive contains only the manuscript source, its matching processed bibliography and the bibliography database. All diagrams are embedded TikZ; no external figure assets are needed.

## Submission settings

Use `submission-metadata.txt` for the title, author and abstract. Recommended primary category: **cs.DB — Databases**. Select **pdfLaTeX** and top-level file `layered-bitemporal-graphs.tex`. The suggested **CC BY 4.0** license matches the existing manuscript declaration; confirm that selection yourself in the submission form. Leave journal reference, journal DOI and report number empty because this package establishes none.

Register as an author and obtain endorsement if arXiv requests it. Upload the ZIP, allow arXiv to compile it, inspect its generated PDF, then complete the metadata and final submission. The accompanying preview PDF is for your comparison; upload the source ZIP. These instructions and metadata files remain outside that ZIP.

## Local verification

The exact archive was extracted into a new directory and compiled with its included `.bbl`, without running BibTeX there or accessing the repository. Its extracted PDF text matches a fresh full LaTeX/BibTeX build. Citations, all six arXiv identifiers, DOI links, page count and warnings were checked. Retained verification report records the original checks and SHA-256 hashes. Duplicate metadata and the separate abstract are retained with that report; the complete text remains in `submission-metadata.txt`.

The title page retains the honest internal revision number and date; these are distinct from arXiv's version numbering. This package has not been uploaded or accepted by arXiv, and its server-side compilation/moderation remain to be completed.

## Reproducibility artifact

The manuscript references `paper/artifact/` and `paper/review-v4/` in the project repository. Their exact public release is still a separate preparation item; this source-only ZIP does not publish that code or verification evidence. The working tree contains uncommitted implementation changes. Publish the matching tested artifact snapshot if you want readers to reproduce those results from a permanent public release. No archived release DOI is claimed here.

## Official instructions

- [Submission overview](https://info.arxiv.org/help/submit/index.html)
- [TeX source and bibliography requirements](https://info.arxiv.org/help/submit_tex.html)
- [Endorsement](https://info.arxiv.org/help/endorsement.html)
- [License choices](https://info.arxiv.org/help/license/index.html)

Requirements checked on 3 October 2026. Local checks do not certify the arXiv server's TeX installation.
