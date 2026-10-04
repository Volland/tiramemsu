"""Rebuild paper and inspect warnings, citation keys, eprints and PDF link annotations."""
from pathlib import Path
import subprocess,json,re,hashlib,sys,tempfile,shutil,os
revision=next((x.split('=',1)[1] for x in sys.argv if x.startswith('--revision=')),'4')
assert revision.isdigit()
paper=Path(__file__).resolve().parents[1];out=paper/('review-v'+revision);out.mkdir(exist_ok=True)
build=Path(tempfile.mkdtemp(prefix='tiramemsu-paper-v'+revision+'-',dir='/tmp'))
runout=out/'builds'/build.name;runout.mkdir(parents=True)
for name in ['layered-bitemporal-graphs.tex','references.bib']:shutil.copy2(paper/name,build/name)
steps=[]
def run(cmd):
 r=subprocess.run(cmd,cwd=build,capture_output=True,text=True)
 steps.append({'command':cmd,'exit_code':r.returncode})
 if r.returncode:
  print(r.stdout[-3000:]);print(r.stderr[-1000:]);raise SystemExit(r.returncode)
 return r
run(['pdflatex','-interaction=nonstopmode','-halt-on-error','layered-bitemporal-graphs.tex'])
run(['bibtex','layered-bitemporal-graphs'])
for i in range(6):
 run(['pdflatex','-interaction=nonstopmode','-halt-on-error','layered-bitemporal-graphs.tex'])
 log=(build/'layered-bitemporal-graphs.log').read_text()
 if i>=1 and all(x not in log for x in ['Rerun to get cross-references right','rerunfilecheck Warning','There were undefined citations','Citation(s) may have changed']):break
pdf=build/'layered-bitemporal-graphs.pdf'
warnings=[l for l in log.splitlines() if any(x in l for x in ['Warning','Overfull','Underfull'])]
(runout/'paper-text.txt').write_text(subprocess.run(['pdftotext','-layout',str(pdf),'-'],capture_output=True,text=True,check=True).stdout)
tex=(paper/'layered-bitemporal-graphs.tex').read_text();bib=(paper/'references.bib').read_text();bbl=(build/'layered-bitemporal-graphs.bbl').read_text()
keys=set(re.findall(r'@\w+\{([^,]+),',bib));cites=set(k.strip() for m in re.finditer(r'\\cite(?:\w*)?(?:\[[^]]*\])*\{([^}]+)\}',tex) for k in m.group(1).split(','));missing=sorted(cites-keys)
assert not missing,missing
ids=re.findall(r'eprint\s*=\s*\{([^}]+)\}',bib)
text=(runout/'paper-text.txt').read_text();flat=re.sub(r'\s+','',text)
assert all(x in flat for x in ids),ids
import fitz
doc=fitz.open(pdf);links=[{'page':i+1,'uri':l['uri']} for i,p in enumerate(doc) for l in p.get_links() if 'uri' in l]
doi_links=[x for x in links if x['uri'].startswith('https://doi.org/')]
assert doi_links,'DOIs not linked'
(out/'pdf-links.json').write_text(json.dumps(links,indent=2))
result={'build_directory':str(build),'render_directory':str(runout),'verified_pdf':str(pdf),'status':'PASS' if not warnings else 'WARN','steps':steps,'warnings':warnings,'pages':len(doc),'bibliography_entries':len(keys),'cited_keys':len(cites),'missing_keys':missing,'rendered_bibitems':len(re.findall(r'\\bibitem',bbl)),'rendered_eprints':ids,'uri_annotations':len(links),'doi_uri_annotations':len(doi_links),'source_sha256':hashlib.sha256(tex.encode()).hexdigest(),'pdf_sha256':hashlib.sha256(pdf.read_bytes()).hexdigest()}
(out/'build-check.json').write_text(json.dumps(result,indent=2));print(json.dumps(result,indent=2))
for i,p in enumerate(doc):p.get_pixmap(matrix=fitz.Matrix(1.1,1.1)).save(runout/f'page-{i+1}.png')

if '--publish' in sys.argv:
 target=paper/('layered-bitemporal-graphs-v'+revision+'.pdf')
 if target.exists():
  target=paper/('layered-bitemporal-graphs-v'+revision+'-'+pdf.stem+'-'+build.name+'.pdf')
 shutil.copy2(pdf,target)
 result['published_pdf']=str(target)
 (out/'build-check.json').write_text(json.dumps(result,indent=2))
 print('Published revision:',target)
