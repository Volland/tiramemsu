#!/usr/bin/env python3
"""Agreement test: SHACL/SPARQL image validators (over the projected row view) versus the
Python `validate`/`decode` of ../check_encodings.py. Needs rdflib and pyshacl; the stdlib-only
checker does not import this file. See README.md."""
import argparse, multiprocessing as mp, os, random, sys, time
from collections import Counter
from itertools import product
from pathlib import Path
from urllib.parse import quote

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent))
import check_encodings as ce
from rdflib import Graph, Literal, Namespace, URIRef, RDF, XSD
from rdflib.namespace import SH
import pyshacl

MG = Namespace('urn:mg:')
SHAPES = Graph().parse(HERE / 'image-shapes.ttl', format='turtle')
QUERIES = {f.stem: f.read_text() for f in HERE.glob('image-*.rq')}
FAMILY_QUERIES = {
    'D': ['image-duplicates', 'image-row-references'],
    'BB': ['image-duplicates', 'image-row-references', 'image-extensionality'],
    'HO': ['image-duplicates', 'image-row-references', 'image-irreflexive'],
    'U': ['image-duplicates', 'image-row-references', 'image-acyclic-member', 'image-extensionality-member'],
    'A': ['image-duplicates', 'image-row-references', 'image-irreflexive'],
}

def project(rows, family, g=None, idx=None):
    """Add the projected row view of one row store to graph g. idx=None uses urn:row:<k>;
    otherwise ids are namespaced per store so many stores can share one graph."""
    g = Graph() if g is None else g
    pre = 'urn:row:' if idx is None else f'urn:row:{idx}.'
    npre = 'urn:node:' if idx is None else f'urn:node:{idx}.'
    store = URIRef(f'urn:store:{0 if idx is None else idx}')
    def term(x):
        if isinstance(x, tuple):
            t = x[0]
            if t == 's': return URIRef(pre + quote(str(x[1])))
            if t == 'n': return URIRef(npre + quote(str(x[1])))
            if t == 'class': return MG[quote(x[1])]
            if t == 'bool': return Literal(x[1], datatype=XSD.boolean) if type(x[1]) is bool else Literal(repr(x[1]))
            if t == 'literal': return Literal(x[1]) if isinstance(x[1], str) else Literal(repr(x[1]), datatype=MG['py'])
        raise ValueError(x)
    tag = MG['Row' + family]
    for sid, (s, p, o) in rows.items():
        r = term(sid)
        g.add((r, RDF.type, tag)); g.add((r, MG.inStore, store))
        g.add((r, MG.rowSubject, term(s))); g.add((r, MG.rowPredicate, MG[quote(p)])); g.add((r, MG.rowObject, term(o)))
        if p == 'type' and isinstance(s, tuple) and s[0] == 'n' and isinstance(o, tuple) and o[0] == 'class':
            g.add((r, MG['class'], term(o)))   # RDF 1.2: ?r rdf:reifies <<( ?n rdf:type ?c )>>
        else:
            g.add((term(s), MG[quote(p)], term(o)))
    return g

def rdf_violations(batch, family):
    """batch: list of row stores -> set of indices rejected by SHACL + SPARQL validators."""
    g = Graph(); owner = {}
    for i, rows in enumerate(batch):
        project(rows, family, g, i)
        for sid in rows:
            owner[URIRef(f'urn:row:{i}.' + quote(str(sid[1])))] = i
    bad = set()
    conforms, rg, _ = pyshacl.validate(g, shacl_graph=SHAPES, inference='none')
    if not conforms:
        for n in rg.objects(None, SH.focusNode):
            bad.add(owner[n])
    for q in FAMILY_QUERIES[family]:
        for (st,) in g.query(QUERIES[q]):
            bad.add(int(str(st).rsplit(':', 1)[1]))
    return bad

def py_ok(rows, family):
    try:
        ce.decode(rows, family); return True
    except ValueError:
        return False

POOL = None

class Tally:
    def __init__(self): self.c = Counter(); self.dis = []; self.sets = {}
    def run(self, name, family, items, mode, chunk=100):
        t0 = time.time()
        parts = [items[lo:lo + chunk] for lo in range(0, len(items), chunk)]
        bads = POOL.starmap(rdf_violations, [(p, family) for p in parts]) if POOL else [rdf_violations(p, family) for p in parts]
        for part, bad in zip(parts, bads):
            for i, rows in enumerate(part):
                py, rdf = py_ok(rows, family), i not in bad
                k = (name, family)
                self.c[k, 'total'] += 1
                self.c[k, 'py_accept' if py else 'py_reject'] += 1
                self.c[k, 'rdf_accept' if rdf else 'rdf_reject'] += 1
                self.c[k, 'agree' if py == rdf else 'disagree'] += 1
                if py != rdf and len(self.dis) < 40:
                    self.dis.append((name, family, py, rdf, rows))
        self.sets[(name, family)] = mode
        print(f'  {name:28s} {family:2s} n={len(items):6d} {mode:10s} {time.time()-t0:6.1f}s', flush=True)

def mutations(rows, rng, n):
    """Seeded single-row perturbations: delete, duplicate, swap object, rename predicate."""
    out, keys = [], list(rows)
    for _ in range(n):
        r = dict(rows); k = rng.choice(keys); kind = rng.choice(['del', 'dup', 'obj', 'pred'])
        s, p, o = rows[k]
        if kind == 'del': del r[k]
        elif kind == 'dup': r[('s', 'dup')] = rows[k]
        elif kind == 'obj': r[k] = (s, p, rng.choice(keys))
        else: r[k] = (s, rng.choice(['in', 'out', 'member', 'src', 'directed', 'attr:z', 'container']), o)
        out.append(r)
    return out

def attributed_stores():
    ca = {'v0': 'Vertex', 'm0': 'MetaVertex', 'e0': 'Edge', 'q0': 'MetaEdge'}
    for ends, flags, frags, attrs in product(product(('v0', 'm0'), repeat=4), product((False, True), repeat=2),
         product(*(ce.subsets(set(ca) - {a}) for a in ('m0', 'q0'))), ce.subsets(ca)):
        roles = [(e, p, ends[2*i+j]) for i, e in enumerate(('e0', 'q0')) for j, p in enumerate(('src', 'tgt'))]
        roles += [(b, 'inGraph', a) for a, ys in zip(('m0', 'q0'), frags) for b in ys]
        yield ce.model(ca, roles, zip(('e0', 'q0'), flags), [(a, 'attr:k', 'value') for a in attrs])

def targeted_bad():
    ca = {'v0': 'Vertex', 'm0': 'MetaVertex', 'e0': 'Edge', 'q0': 'MetaEdge'}
    seed = ce.encode(next(m for m in ce.sources(True) if len(m.classes) == 3))
    hs = [sid for sid, (_, p, _) in seed.items() if p == 'type']
    bad = [('HO', seed | {('s', 'extra'): (hs[0], 'relatedTo', hs[1])}),
           ('HO', seed | {('s', 'extra'): next(c for c in seed.values() if c[1] == 'in')}),
           ('HO', seed | {('s', 'extra'): (('s', 'absent'), 'in', hs[0])}),
           ('HO', seed | {('s', 'extra'): (('s', 'extra'), 'attr:k', ('literal', 'value'))})]
    ma = ce.model(ca, [('e0', 'src', 'v0'), ('e0', 'tgt', 'm0'), ('q0', 'src', 'v0'), ('q0', 'tgt', 'm0')], [('e0', True), ('q0', False)])
    rows = ce.encode(ma); h = {s[1]: sid for sid, (s, p, o) in rows.items() if p == 'type'}
    bad += [('A', {sid: c for sid, c in rows.items() if c[1] != 'directed'}),
            ('A', rows | {('s', 'extra'): (h['e0'], 'src', h['m0'])}),
            ('A', rows | {('s', 'extra'): (h['m0'], 'inGraph', h['m0'])}),
            ('A', rows | {('s', 'extra'): (h['v0'], 'inGraph', h['e0'])}),
            ('A', rows | {('s', 'extra'): (h['e0'], 'directed', ('literal', 'yes'))})]
    return bad

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--seed', type=int, default=2025)
    ap.add_argument('--candidate-sample', type=int, default=0, help='0 = all 65,536 role-subset stores under full SHACL')
    ap.add_argument('--attributed-sample', type=int, default=2000)
    ap.add_argument('--mutations', type=int, default=3, help='mutations per sampled store')
    ap.add_argument('--jobs', type=int, default=os.cpu_count() or 1, help='worker processes (SHACL is per-chunk parallel)')
    a = ap.parse_args()
    global POOL
    POOL = mp.get_context('fork').Pool(a.jobs) if a.jobs > 1 else None
    rng = random.Random(a.seed); T = Tally(); t0 = time.time()
    print('(a) enumerated sources'); 
    for hi in (False, True):
        ms = list(ce.sources(hi)); st = [ce.encode(m) for m in ms]
        fams = ['D', 'BB'] + (['HO'] if hi else [])
        for f in fams: T.run('sources(higher=%s)' % hi, f, st, 'exhaustive')
    print('(a2) HO-source stores under BB-extensionality neighbours: random single-row mutations')
    base = [ce.encode(m) for m in ce.sources(True)]
    for f in ('D', 'BB', 'HO'):
        samp = rng.sample(base, 400)
        T.run('mutated sources', f, [r for s in samp for r in mutations(s, rng, a.mutations)], 'sampled')
    print('(b) recursive member candidates')
    c = {'v0': 'Vertex', 'v1': 'Vertex', 'e0': 'Edge', 'e1': 'Edge'}
    ms = [ce.model(c, [(e, 'member', x) for e, ys in zip(('e0', 'e1'), mem) for x in ys])
          for mem in product(*(ce.subsets(set(c) - {e}, True) for e in ('e0', 'e1')))]
    st = [ce.encode(m) for m in ms]
    T.run('recursive member sets', 'U', st, 'exhaustive')
    ms2 = [ce.model(c, [(e, 'member', x) for e, ys in zip(('e0', 'e1'), mem) for x in ys])   # self-members and empties too
           for mem in product(*(ce.subsets(set(c)) for e in ('e0', 'e1')))]
    T.run('all member subsets (incl. self)', 'U', [ce.encode(m) for m in ms2], 'exhaustive')
    print('(c) attributed')
    allA = list(attributed_stores())
    samp = rng.sample(allA, a.attributed_sample)
    sa = [ce.encode(m) for m in samp]
    T.run('attributed enumeration', 'A', sa, 'sampled')
    T.run('mutated attributed', 'A', [r for s in sa[:600] for r in mutations(s, rng, a.mutations)], 'sampled')
    print('(d) 65,536 role-subset candidates (family HO)')
    possible = [(e, p, b) for e in ('e0', 'e1') for p in ('in', 'out') for b in c]
    cands = ce.subsets(possible)
    assert len(cands) == 65536
    if a.candidate_sample:
        cands = rng.sample(cands, a.candidate_sample); mode = 'sampled'
    else: mode = 'exhaustive'
    T.run('role-subset candidates', 'HO', [ce.encode(ce.model(c, r)) for r in cands], mode, chunk=250)
    print('(e) targeted malformed images')
    for f, rows in targeted_bad(): T.run('targeted malformed', f, [rows], 'exhaustive')
    print(f'\n== summary ({time.time()-t0:.0f}s) ==')
    keys = sorted({k for k, _ in T.c})
    tot = Counter()
    for k in keys:
        n = T.c[k, 'total']
        print(f'{k[0]:34s} {k[1]:2s} {T.sets[k]:10s} total={n:6d} py acc/rej={T.c[k,"py_accept"]}/{T.c[k,"py_reject"]} '
              f'rdf acc/rej={T.c[k,"rdf_accept"]}/{T.c[k,"rdf_reject"]} agree={T.c[k,"agree"]} disagree={T.c[k,"disagree"]}')
        tot[k[1], 'agree'] += T.c[k, 'agree']; tot[k[1], 'disagree'] += T.c[k, 'disagree']
    for f in ('D', 'BB', 'HO', 'U', 'A'):
        print(f'family {f}: agree={tot[f,"agree"]} disagree={tot[f,"disagree"]}')
    for name, f, py, rdf, rows in T.dis[:10]:
        print('DISAGREE', name, f, 'python', py, 'rdf', rdf); print('   ', rows)
    sys.exit(1 if any(v for (f, k), v in tot.items() if k == 'disagree') else 0)

if __name__ == '__main__':
    main()
