# Zenodo deposit

Upload `../arxiv/revision-4/layered-bitemporal-graphs-arxiv-preview-v4.pdf` (the revision-4 PDF) and use `metadata.json` for the form fields.

## Web upload

1. Sign in at zenodo.org, choose New upload, and drop in the PDF.
2. Resource type Publication / Preprint; copy title, description, keywords, CC BY 4.0 and version 4 from `metadata.json`.
3. Optionally add your ORCID to the creator, and the arXiv identifier as a related identifier (`isVersionOf`) once it exists.
4. Review, then Publish. Publishing mints a permanent DOI, so check the PDF first.

Optionally attach the LaTeX source ZIP (`../arxiv/revision-4/layered-bitemporal-graphs-arxiv-v4.zip`) as a second file.

## After publishing

Add the DOI to the paper page (`site/paper/index.html`), its BibTeX entry and `../README.md`. Later revisions go in as a new version of the same record, which keeps one concept DOI.
