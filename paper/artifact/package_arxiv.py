"""Prepare and verify an arXiv source upload, with guidance kept outside the ZIP."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import zipfile

PAPER = Path(__file__).resolve().parents[1]
MAIN = 'layered-bitemporal-graphs'


def main():
    tex = (PAPER / (MAIN + '.tex')).read_text()
    revision = re.search(r'Revision (\d+),', tex).group(1)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=PAPER / 'arxiv' / ('revision-' + revision))
    args = parser.parse_args()
    out = args.output.resolve()
    if out.exists():
        raise SystemExit('Output already exists; select a fresh directory with --output.')
    out.mkdir(parents=True)
    work = Path(tempfile.mkdtemp(prefix='tiramemsu-arxiv-', dir='/tmp'))
    build = work / 'build'
    build.mkdir()
    for name in [MAIN + '.tex', 'references.bib']:
        shutil.copy2(PAPER / name, build / name)
    steps = []

    def run(cmd, cwd):
        result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
        steps.append({'command':cmd, 'stage':cwd.name, 'exit_code':result.returncode})
        if result.returncode:
            (out / 'failed-build.log').write_text(result.stdout + result.stderr)
            raise SystemExit('Build failed; see failed-build.log.')
        return result.stdout

    latex = ['pdflatex', '-interaction=nonstopmode', '-halt-on-error', MAIN + '.tex']
    run(latex, build)
    run(['bibtex', MAIN], build)
    for _ in range(3):
        run(latex, build)
    upload = out / ('layered-bitemporal-graphs-arxiv-v' + revision + '.zip')
    names = [MAIN + '.tex', MAIN + '.bbl', 'references.bib']
    with zipfile.ZipFile(upload, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        for name in names:
            archive.write(build / name, name)
    # The compilation below can see ONLY the files extracted from the upload.
    extracted = work / 'extracted'
    extracted.mkdir()
    with zipfile.ZipFile(upload) as archive:
        assert archive.namelist() == names
        assert archive.testzip() is None
        archive.extractall(extracted)
    for _ in range(3):
        run(latex, extracted)
    log = (extracted / (MAIN + '.log')).read_text()
    warnings = [line for line in log.splitlines() if any(x in line for x in ['Warning', 'Overfull', 'Underfull'])]
    assert not warnings, warnings
    assert (extracted / (MAIN + '.bbl')).read_bytes() == (build / (MAIN + '.bbl')).read_bytes()
    def pdf_text(folder):
        return run(['pdftotext', '-layout', MAIN + '.pdf', '-'], folder)
    assert pdf_text(build) == pdf_text(extracted), 'ZIP-only build differs from full build'
    bib = (build / 'references.bib').read_text()
    keys = set(re.findall(r'@\w+\{([^,]+),', bib))
    cites = {k.strip() for m in re.finditer(r'\\cite\w*(?:\[[^]]*\])*\{([^}]+)\}', tex) for k in m[1].split(',')}
    bbl = (build / (MAIN + '.bbl')).read_text()
    assert cites <= keys
    assert len(re.findall(r'\\bibitem', bbl)) == len(cites)
    text = pdf_text(extracted)
    assert all(x in re.sub(r'\s+', '', text) for x in re.findall(r'eprint\s*=\s*\{([^}]+)\}', bib))
    import fitz
    doc = fitz.open(extracted / (MAIN + '.pdf'))
    links = [link['uri'] for page in doc for link in page.get_links() if 'uri' in link]
    assert any(link.startswith('https://doi.org/') for link in links)
    preview = out / ('layered-bitemporal-graphs-arxiv-preview-v' + revision + '.pdf')
    shutil.copy2(extracted / (MAIN + '.pdf'), preview)
    title = 'Layered Bitemporal Graphs: Statement Identity, Two Clocks, and Metagraphs as Tags'
    abstract = re.search(r'\\begin\{abstract\}(.*?)\\end\{abstract\}', tex, re.S)[1]
    abstract = ' '.join(abstract.split())
    assert '\\' not in abstract, 'Convert abstract macros to metadata-safe text'
    metadata = {'title':title, 'authors':'Volodymyr Pavlyshyn', 'affiliation':'Independent researcher',
        'abstract':abstract, 'recommended_primary_category':'cs.DB (Databases)',
        'recommended_cross_lists':[], 'suggested_license':'CC BY 4.0',
        'comments':str(len(doc)) + ' pages. Original research preprint.',
        'journal_reference':'', 'doi':'', 'report_number':'',
        'top_level_tex':MAIN + '.tex', 'compiler':'pdfLaTeX',
        'internal_manuscript_revision':revision, 'status':'Prepared locally; not submitted to arXiv.'}
    (out / 'submission-metadata.json').write_text(json.dumps(metadata, indent=2, ensure_ascii=False) + '\n')
    (out / 'submission-metadata.txt').write_text('\n\n'.join(k.replace('_',' ').title() + ':\n' + str(v) for k,v in metadata.items() if isinstance(v,str)) + '\n')
    (out / 'abstract.txt').write_text(abstract + '\n')
    instructions = f'''# arXiv upload package — internal manuscript revision {revision}

Upload **{upload.name}** through your arXiv author account. The archive contains only the manuscript source, its matching processed bibliography and the bibliography database. All diagrams are embedded TikZ; no external figure assets are needed.

## Submission settings

Use `submission-metadata.txt` or `submission-metadata.json` for the title, author and abstract. Recommended primary category: **cs.DB — Databases**. Select **pdfLaTeX** and top-level file `{MAIN}.tex`. The suggested **CC BY 4.0** license matches the existing manuscript declaration; confirm that selection yourself in the submission form. Leave journal reference, journal DOI and report number empty because this package establishes none.

Register as an author and obtain endorsement if arXiv requests it. Upload the ZIP, allow arXiv to compile it, inspect its generated PDF, then complete the metadata and final submission. The accompanying preview PDF is for your comparison; upload the source ZIP. These instructions and metadata files remain outside that ZIP.

## Local verification

The exact archive was extracted into a new directory and compiled with its included `.bbl`, without running BibTeX there or accessing the repository. Its extracted PDF text matches a fresh full LaTeX/BibTeX build. Citations, all six arXiv identifiers, DOI links, page count and warnings were checked. `package-check.json` records checks and SHA-256 hashes.

The title page retains the honest internal revision number and date; these are distinct from arXiv's version numbering. This package has not been uploaded or accepted by arXiv, and its server-side compilation/moderation remain to be completed.

## Reproducibility artifact

The manuscript references `paper/artifact/` and `paper/review-v4/` in the project repository. Their exact public release is still a separate preparation item; this source-only ZIP does not publish that code or verification evidence. The working tree contains uncommitted implementation changes. Publish the matching tested artifact snapshot if you want readers to reproduce those results from a permanent public release. No archived release DOI is claimed here.

## Official instructions

- [Submission overview](https://info.arxiv.org/help/submit/index.html)
- [TeX source and bibliography requirements](https://info.arxiv.org/help/submit_tex.html)
- [Endorsement](https://info.arxiv.org/help/endorsement.html)
- [License choices](https://info.arxiv.org/help/license/index.html)

Requirements checked on 3 October 2026. Local checks do not certify the arXiv server's TeX installation.
'''
    (out / 'README.md').write_text(instructions)
    result = {'status':'PASS', 'date':'2026-10-03', 'internal_revision':revision, 'archive':upload.name,
        'zip_entries':names, 'zip_integrity':'PASS', 'zip_only_build':'PASS', 'bibtex_required_for_zip_only_build':False,
        'zip_only_text_matches_fresh_full_build':True, 'pages':len(doc), 'warnings':warnings,
        'bibliography_entries':len(keys), 'cited_keys':len(cites), 'rendered_bibitems':len(cites),
        'uri_annotations':len(links), 'doi_uri_annotations':sum(x.startswith('https://doi.org/') for x in links),
        'tex_live':run(['pdflatex','--version'], extracted).splitlines()[0], 'steps':steps,
        'source_sha256':hashlib.sha256(tex.encode()).hexdigest(),
        'archive_member_sha256':{name:hashlib.sha256((build/name).read_bytes()).hexdigest() for name in names},
        'deliverable_sha256':{x.name:hashlib.sha256(x.read_bytes()).hexdigest() for x in sorted(out.iterdir()) if x.is_file()},
        'server_compilation':'Not performed', 'arxiv_submission':'Not performed'}
    (out / 'package-check.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'status':'PASS','directory':str(out),'upload':str(upload),'pages':len(doc),'warnings':warnings,'cited_entries':len(cites)},indent=2))


if __name__ == '__main__':
    main()
