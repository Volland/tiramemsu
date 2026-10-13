# Zenodo deposit

Published 13 October 2026: concept DOI [10.5281/zenodo.23335652](https://doi.org/10.5281/zenodo.23335652), revision 4 as version DOI 10.5281/zenodo.23335653.

One record with two files:

1. `../statement-as-one-record.pdf`: the revision-4 paper.
2. `statement-as-one-record-r4-validation.zip`: the validation archive. Build it with `python3 build_archive.py`. The script refuses a stale validation or a changed engine trace and builds the Lean project first. The archive holds the LaTeX source, `validate.py` with its results and counts, the engine trace and the engine test that writes it, the Lean development and `SHA256SUMS`. Its own README explains how to reproduce.

Copy the form fields from `metadata.json`: title with subtitle, description, keywords, CC BY 4.0, version 4 and publication date 12 October 2026.

## Web upload

1. Sign in at zenodo.org, choose New upload, and drop in both files.
2. Resource type: Publication / Preprint. Copy the fields from `metadata.json`. Optionally add your ORCID to the creator.
3. Related works:
   - cites the first paper, doi 10.5281/zenodo.23139228;
   - cites the second paper, doi 10.5281/zenodo.23167938 (version 4 cited it by repository URL; `metadata.json` now uses the DOI for later versions);
   - cites Gelling et al., arXiv:2304.13097;
   - the GitHub repository is supplementary software.
4. Review, then Publish. Publishing mints a permanent DOI, so check the PDF and unzip the archive first.

## After publishing

Add the DOI to this folder's parent `README.md`, to the BibTeX entries that cite this paper and to the top-level `README.md`. Later revisions go in as a new version of the same record, which keeps one concept DOI. Rebuild the archive for each version.
