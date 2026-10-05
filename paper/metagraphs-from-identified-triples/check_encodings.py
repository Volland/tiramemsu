#!/usr/bin/env python3
"""Deterministic bounded checks. Python standard library only; see README.md."""
from dataclasses import dataclass
from collections import Counter, defaultdict, deque
from itertools import combinations, product
from pathlib import Path
import argparse
import json

KINDS = ('Vertex', 'MetaVertex', 'Edge', 'MetaEdge')
RESERVED = {'type', 'in', 'out', 'member', 'src', 'tgt', 'directed', 'container', 'inGraph'}

@dataclass(frozen=True)
class Model:
    classes: tuple
    roles: frozenset = frozenset()
    flags: frozenset = frozenset()
    attrs: frozenset = frozenset()

def model(classes, roles=(), flags=(), attrs=()):
    return Model(tuple(sorted(classes.items())), frozenset(roles), frozenset(flags), frozenset(attrs))

def subsets(xs, nonempty=False):
    xs = sorted(xs)
    return [frozenset(c) for n in range(int(nonempty), len(xs)+1) for c in combinations(xs, n)]

def encode(m):
    rows, handles = {}, {}
    def add(content):
        sid = ('s', len(rows))
        rows[sid] = content
        return sid
    for a, k in m.classes:
        handles[a] = add((('n', a), 'type', ('class', k)))
    for a, p, b in sorted(m.roles):
        add((handles[a], p, handles[b]))
    for a, b in sorted(m.flags):
        add((handles[a], 'directed', ('bool', b)))
    for a, k, v in sorted(m.attrs):
        add((handles[a], k, ('literal', v)))
    return rows

def is_stmt(term):
    return isinstance(term, tuple) and term[0] == 's'

def ranks(rows):
    memo, active = {}, set()
    def visit(sid):
        if sid in active:
            raise ValueError('reference cycle')
        if sid in memo:
            return memo[sid]
        active.add(sid)
        s, _, o = rows[sid]
        refs = [x for x in (s,o) if is_stmt(x)]
        if sid in refs or any(x not in rows for x in refs):
            raise ValueError('self or dangling reference')
        result = 1+max(map(visit, refs)) if refs else 0
        active.remove(sid)
        memo[sid] = result
        return result
    for sid in rows:
        visit(sid)
    return memo

def scan(rows):
    """Scan contents without using encoder identifier names or insertion order."""
    ranks(rows)
    handles, classes, nodes = {}, {}, set()
    for sid, (s,p,o) in rows.items():
        if p == 'type':
            if not isinstance(s, tuple) or s[0] != 'n' or o not in {('class',k) for k in KINDS} or s in nodes:
                raise ValueError('anchor')
            nodes.add(s)
            handles[sid] = s[1]
            classes[s[1]] = o[1]
    roles, flags, attrs = [], [], []
    for sid, (s,p,o) in rows.items():
        if sid in handles:
            continue
        if s in nodes or o in nodes or s not in handles:
            raise ValueError('fresh node reused or field subject')
        if p in {'in','out','member','src','tgt','inGraph'}:
            if o not in handles:
                raise ValueError('role object')
            roles.append((handles[s],p,handles[o]))
        elif p == 'directed':
            if not isinstance(o,tuple) or len(o)!=2 or o[0]!='bool' or type(o[1]) is not bool:
                raise ValueError('direction')
            flags.append((handles[s],o[1]))
        elif p not in RESERVED and isinstance(o,tuple) and o[0]=='literal':
            attrs.append((handles[s],p,o[1]))
        else:
            raise ValueError('unknown field')
    if any(len(x)!=len(set(x)) for x in (roles,flags,attrs)):
        raise ValueError('duplicate occurrence')
    return model(classes,roles,flags,attrs)

def acyclic(nodes, adj):
    active, done = set(), set()
    def visit(a):
        if a in active:
            return False
        if a in done:
            return True
        active.add(a)
        if any(not visit(b) for b in adj.get(a,()) if b in nodes):
            return False
        active.remove(a)
        done.add(a)
        return True
    return all(visit(a) for a in nodes)

def validate(m, family, drop=()):
    """Validate a model against an image family; `drop` removes named constraints (ablation)."""
    keep = lambda name: name not in drop
    c = dict(m.classes)
    es = {a for a,k in m.classes if k=='Edge'}
    fields = defaultdict(set)
    if family in {'D','BB','HO','U'}:
        if any(k not in {'Vertex','Edge'} for k in c.values()):
            raise ValueError('classes')
        if (m.flags or m.attrs) and keep('D5'):
            raise ValueError('extra properties')
        for a,p,b in m.roles:
            if (a not in es and keep('U1' if family=='U' else 'D1')) or b not in c:
                raise ValueError('role type')
            if family=='U':
                if p!='member' and keep('U1'):
                    raise ValueError('member role required')
            elif p not in {'in','out'} or (family in {'D','BB'} and c[b]!='Vertex' and keep('D2')) \
                    or (family=='HO' and a==b and keep('H2')):
                raise ValueError('directed role type')
            fields[a,p].add(b)
        if family=='U':
            if keep('U2') and any(not fields[e,'member'] for e in es):
                raise ValueError('empty members')
            if keep('U3') and not acyclic(es,{e:fields[e,'member'] for e in es}):
                raise ValueError('cyclic members')
            sig = [frozenset(fields[e,'member']) for e in es]
        else:
            for p,name in (('in','D3'),('out','D4')):
                if keep(name) and any(not fields[e,p] for e in es):
                    raise ValueError('empty ends')
            sig = [(frozenset(fields[e,'in']),frozenset(fields[e,'out'])) for e in es]
        if (family=='BB' and keep('B1') or family=='U' and keep('U4')) and len(sig)!=len(set(sig)):
            raise ValueError('extensionality')
    elif family=='A':
        es = {a for a,k in m.classes if k in {'Edge','MetaEdge'}}
        vs = {a for a,k in m.classes if k in {'Vertex','MetaVertex'}}
        cs = {a for a,k in m.classes if k in {'MetaVertex','MetaEdge'}}
        card = Counter()
        for a,p,b in m.roles:
            if p in {'src','tgt'} and a in es and (b in vs or (not keep('A4') and b in c)):
                card[a,p]+=1
            elif p=='inGraph' and a in c and (b in cs or (not keep('A5') and b in c)) and (a!=b or not keep('A6')):
                pass
            else:
                raise ValueError('attributed role')
        for p,name in (('src','A1'),('tgt','A2')):
            if keep(name) and any(card[e,p]!=1 for e in es):
                raise ValueError('endpoint cardinality')
        fc = Counter(a for a,b in m.flags)
        if keep('A3') and (set(fc)!=es or any(fc[e]!=1 for e in es)):
            raise ValueError('direction cardinality')
        if any(a not in c or (k in RESERVED and keep('A7')) for a,k,v in m.attrs):
            raise ValueError('attribute')
    else:
        raise ValueError('family')
    return m

def decode(rows,family,drop=()):
    return validate(scan(rows),family,drop)

def renamed(rows):
    mapping = {sid:('s','renamed'+str(i)) for i,sid in enumerate(reversed(rows))}
    return {mapping[sid]:tuple(mapping.get(x,x) for x in content) for sid,content in reversed(list(rows.items()))}

def roundtrip(m,family):
    validate(m,family)
    rows=encode(m)
    assert decode(rows,family)==m
    assert encode(decode(renamed(rows),family))==rows
    assert len(rows)==len(m.classes)+len(m.roles)+len(m.flags)+len(m.attrs)
    assert max(ranks(rows).values(),default=0)<=1

def sources(higher=False):
    for n in range(3):
        c={'v0':'Vertex','v1':'Vertex'} | {f'e{i}':'Edge' for i in range(n)}
        opts=[list(product(subsets(set(c)-{e} if higher else {'v0','v1'},True),repeat=2)) for e in [f'e{i}' for i in range(n)]]
        for pairs in product(*opts):
            yield model(c,[(f'e{i}',p,y) for i,pair in enumerate(pairs) for p,ys in zip(('in','out'),pair) for y in ys])

def recognize(m,B,C,F):
    adj,rev=defaultdict(list),defaultdict(list)
    I,O=set(),set()
    for e,p,v in m.roles:
        if e not in F:
            continue
        a,b=(('v',v),('e',e)) if p=='in' else (('e',e),('v',v))
        adj[a].append(b); rev[b].append(a)
        (I if p=='in' else O).add(v)
    def reach(g,starts):
        seen=set(starts); q=deque(starts)
        while q:
            for b in g[q.popleft()]:
                if b not in seen:
                    seen.add(b); q.append(b)
        return seen
    front=reach(adj,[('v',x) for x in B]); back=reach(rev,[('v',x) for x in C])
    return all(('e',e) in front and ('e',e) in back for e in F) and I-O<=B and C<=O

def oracle(m,B,C,F):
    """Independent bounded edge-sequence enumeration, including repetitions."""
    I={e:{v for a,p,v in m.roles if a==e and p=='in'} for e in F}
    O={e:{v for a,p,v in m.roles if a==e and p=='out'} for e in F}
    covered=set()
    for n in range(1,2*len(F)+2):
        for seq in product(sorted(F),repeat=n):
            if I[seq[0]]&B and O[seq[-1]]&C and all(O[a]&I[b] for a,b in zip(seq,seq[1:])):
                covered.update(seq)
        if covered==set(F):
            break
    ins=set().union(*I.values()) if I else set()
    outs=set().union(*O.values()) if O else set()
    return covered==set(F) and ins-outs<=B and C<=outs

def reject(rows,family):
    try:
        decode(rows,family)
    except ValueError:
        return
    raise AssertionError(('accepted invalid image',family,rows))

def compact_checks():
    count=0
    def extend(rows,depths,n):
        nonlocal count
        if n==0:
            assert ranks(rows)==depths
            count+=1; return
        sid=('s','edge'+str(n))
        for a,b in product(rows,repeat=2):
            extend(rows | {sid:(a,'binary',b)},depths | {sid:1+max(depths[a],depths[b])},n-1)
    rows=encode(model({'v0':'Vertex','v1':'Vertex'}))
    extend(rows,{sid:0 for sid in rows},4)
    return count

def temporal_checks():
    # Fixed immutable components; snapshots select all or none of mandatory fields.
    m=model({'v0':'Vertex','m0':'MetaVertex','e0':'Edge'},[('e0','src','v0'),('e0','tgt','m0')],[('e0',True)])
    base=encode(m)
    h={s[1]:sid for sid,(s,p,o) in base.items() if p=='type'}
    groups={a:{h[a]} for a,k in m.classes}
    for sid,(s,p,o) in base.items():
        if p!='type':
            groups['e0'].add(sid)
    extra={
        'membership':(h['v0'],'inGraph',h['m0']),
        'attribute':(h['v0'],'attr:k',('literal','value')),
        'assessment':(('s','membership'),'confidence',('literal','0.7')),
        'audit':(('s','assessment'),'derivedFrom',('literal','source'))}
    full=base | {('s',a):c for a,c in extra.items()}
    groups.update({a:{('s',a)} for a in extra})
    deps={'v0':set(),'m0':set(),'e0':{'v0','m0'},'membership':{'v0','m0'},'attribute':{'v0'},'assessment':{'membership'},'audit':{'assessment'}}
    count=0
    for keep in subsets(groups):
        if any(not deps[a]<=keep for a in keep):
            continue
        ids=set().union(*(groups[a] for a in keep)) if keep else set()
        lhs={sid:c for sid,c in full.items() if sid in ids}
        ranks(lhs)
        roles=[r for r in m.roles if r[0] in keep]
        if 'membership' in keep:
            roles.append(('v0','inGraph','m0'))
        chosen=model({a:k for a,k in m.classes if a in keep},roles,[r for r in m.flags if r[0] in keep],
                     [('v0','attr:k','value')] if 'attribute' in keep else [])
        rhs=encode(chosen)
        structural={sid:c for sid,c in lhs.items() if sid not in {('s','assessment'),('s','audit')}}
        assert encode(decode(structural,'A'))==rhs
        if 'assessment' in keep:
            bearer=next(sid for sid,(_,p,_) in rhs.items() if p=='inGraph')
            rhs[('s','newassessment')]=(bearer,'confidence',('literal','0.7'))
        if 'audit' in keep:
            rhs[('s','newaudit')]=(('s','newassessment'),'derivedFrom',('literal','source'))
        def rooted(rows,sid):
            s,p,o=rows[sid]
            return (rooted(rows,s) if s in rows else s,p,rooted(rows,o) if o in rows else o)
        assert Counter(rooted(lhs,s) for s in lhs)==Counter(rooted(rhs,s) for s in rhs)
        count+=1
    return count

def run():
    counts=Counter()
    for m in sources():
        roundtrip(m,'D'); counts['directed_roundtrips']+=1
        try:
            validate(m,'BB')
        except ValueError:
            pass
        else:
            roundtrip(m,'BB'); counts['bb_roundtrips']+=1
        es={a for a,k in m.classes if k=='Edge'}
        for B,C,F in product(subsets({'v0','v1'}),subsets({'v0','v1'}),subsets(es)):
            assert recognize(m,B,C,F)==oracle(m,B,C,F),(m,B,C,F)
            counts['metapath_queries']+=1
    for m in sources(True):
        roundtrip(m,'HO'); counts['higher_order_roundtrips']+=1
    c={'v0':'Vertex','v1':'Vertex','e0':'Edge','e1':'Edge'}
    accepted_stores=set()
    possible=[(e,p,b) for e in ('e0','e1') for p in ('in','out') for b in c]
    for roles in subsets(possible):
        rows=encode(model(c,roles)); counts['higher_order_candidate_stores']+=1
        try:
            decoded=decode(rows,'HO')
        except ValueError:
            counts['higher_order_rejected_candidates']+=1
        else:
            assert encode(decoded)==rows
            accepted_stores.add(frozenset(rows.values()))
            counts['higher_order_accepted_candidates']+=1
    two_edge={frozenset(encode(m).values()) for m in sources(True) if len(m.classes)==4}
    assert accepted_stores==two_edge and len(two_edge)==counts['higher_order_accepted_candidates']
    assert counts['higher_order_roundtrips']==1+9+len(two_edge)
    counts['accepted_equal_two_edge_sources']=len(two_edge)
    for members in product(*(subsets(set(c)-{e},True) for e in ('e0','e1'))):
        m=model(c,[(e,'member',a) for e,ys in zip(('e0','e1'),members) for a in ys])
        counts['recursive_candidates']+=1
        try:
            validate(m,'U')
        except ValueError:
            reject(encode(m),'U')
        else:
            roundtrip(m,'U'); counts['recursive_roundtrips']+=1
            fields={e:{b for a,p,b in m.roles if a==e} for e in ('e0','e1')}
            def depth(e):
                sub=fields[e]&fields.keys()
                return 1+max(depth(b) for b in sub) if sub else 0
            def paths(a):
                return [1+n for b in fields[a] for n in paths(b)] if a in fields else [0]
            assert all(depth(e)==max(paths(e))-1 for e in fields)
    ca={'v0':'Vertex','m0':'MetaVertex','e0':'Edge','q0':'MetaEdge'}
    for ends,flags,frags,attrs in product(product(('v0','m0'),repeat=4),product((False,True),repeat=2),
         product(*(subsets(set(ca)-{a}) for a in ('m0','q0'))),subsets(ca)):
        roles=[(e,p,ends[2*i+j]) for i,e in enumerate(('e0','q0')) for j,p in enumerate(('src','tgt'))]
        roles += [(b,'inGraph',a) for a,ys in zip(('m0','q0'),frags) for b in ys]
        m=model(ca,roles,zip(('e0','q0'),flags),[(a,'attr:k','value') for a in attrs])
        roundtrip(m,'A'); counts['attributed_roundtrips']+=1
    seed=encode(next(m for m in sources(True) if len(m.classes)==3))
    hs=[sid for sid,(_,p,_) in seed.items() if p=='type']
    bad=[('HO',seed | {('s','extra'):(hs[0],'relatedTo',hs[1])}),
         ('HO',seed | {('s','extra'):next(c for c in seed.values() if c[1]=='in')}),
         ('HO',seed | {('s','extra'):(('s','absent'),'in',hs[0])}),
         ('HO',seed | {('s','extra'):(('s','extra'),'attr:k',('literal','value'))})]
    ma=model(ca,[('e0','src','v0'),('e0','tgt','m0'),('q0','src','v0'),('q0','tgt','m0')],[('e0',True),('q0',False)])
    rows=encode(ma); h={s[1]:sid for sid,(s,p,o) in rows.items() if p=='type'}
    bad += [('A',{sid:c for sid,c in rows.items() if c[1]!='directed'}),
            ('A',rows | {('s','extra'):(h['e0'],'src',h['m0'])}),
            ('A',rows | {('s','extra'):(h['m0'],'inGraph',h['m0'])}),
            ('A',rows | {('s','extra'):(h['v0'],'inGraph',h['e0'])}),
            ('A',rows | {('s','extra'):(h['e0'],'directed',('literal','yes'))})]
    for family,rows in bad:
        reject(rows,family); counts['targeted_image_rejections']+=1
    counts['compact_depth_cases']=compact_checks()
    counts['temporal_closed_selections']=temporal_checks()
    rows=encode(model({'v0':'Vertex','v1':'Vertex'}))
    rows |= {('s','member'):(('s',0),'inGraph',('s',1)),
             ('s','low'):(('s',0),'confidence',('literal','0.9')),
             ('s','high'):(('s','member'),'confidence',('literal','0.7'))}
    assert ranks(rows)[('s','low')]==1 and ranks(rows)[('s','high')]==2
    counts['rank_semantics_counterexamples']=1
    import check_extensions
    ext_bounds=check_extensions.run_extensions(counts)
    return {'status':'passed','bounds':{
        'directed':'2 vertices, 0..2 edges; all nonempty endpoint sets',
        'higher_order':'2 vertices, 0..2 edges; all nonempty self-excluding endpoint sets',
        'candidate_stores':'2 vertices, 2 edges; all subsets of 16 roles including self-end roles',
        'recursive':'2 vertices, 2 edges; all nonempty self-excluding member sets; acyclicity/extensionality filter',
        'attributed':'one atom per class; 2^4 endpoint, 2^2 direction, 2^6 fragment, 2^4 attribute-presence assignments',
        'metapath':'all B,C,F subsets for directed sources; edge-sequence bound 2|F|+1',
        'compact':'2 vertices, 4 ordered edges; all earlier-endpoint pairs',
        'temporal':'all subsets of 7 fixed components; dependency-closure filter',
        'revision2':ext_bounds},
        'counts':dict(sorted(counts.items())),
        'limits':'Bounded and sampled checks; Lean covers only the directed/higher-order/BB roundtrip under canonical naming; no engine benchmarks; temporal histories are sampled, not exhaustive.'}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,default=Path('validation-results.json'))
    args=parser.parse_args()
    result=run(); c=result['counts']
    args.output.write_text(json.dumps(result,indent=2)+'\n')
    labels=[('Exhaustive: directed roundtrips (identified edges)','directed_roundtrips'),
        ('Exhaustive: Basu--Blanning roundtrips (extensional)','bb_roundtrips'),
        ('Exhaustive: higher-order roundtrips ($1+9+2{,}401$ sources)','higher_order_roundtrips'),
        ('Exhaustive: higher-order candidate stores','higher_order_candidate_stores'),
        ('Accepted / rejected candidates ($2{,}401$ = two-edge sources)',None),
        ('Exhaustive: recursive candidates / accepted roundtrips','recursive'),
        ('Exhaustive: four-class attributed roundtrips','attributed_roundtrips'),
        ('Exhaustive: metapath query/oracle comparisons','metapath_queries'),
        ('Exhaustive: compact depth configurations','compact_depth_cases'),
        ('Exhaustive: dependency-closed temporal selections','temporal_closed_selections'),
        ('Targeted malformed-image rejections','targeted_image_rejections'),
        ('Necessity witnesses (one per constraint)','necessity_witnesses'),
        ('Sampled: random valid models, roundtrip','random_roundtrips'),
        ('Sampled: mutated stores accepted / rejected','mutated'),
        ('Sampled: strata and cascade checks (random $\\NF4$)','strata_checks'),
        ('Exhaustive: firing-closure cases (3 vertices, $\\le2$ edges)','firing_exhaustive'),
        ('Sampled: firing-closure cases with sequence oracle','firing_random'),
        ('Metapaths with coincidence antecedent / strict gap','firing'),
        ('Sampled: snapshot points, Policy R / Policy S','temporal'),
        ('Sampled: version chains, violations rejected','versions')]
    lines=[]
    for label,key in labels:
        if key is None:
            value=f"{c['higher_order_accepted_candidates']:,} / {c['higher_order_rejected_candidates']:,}"
        elif key=='recursive':
            value=f"{c['recursive_candidates']:,} / {c['recursive_roundtrips']:,}"
        elif key=='mutated':
            value=f"{c['mutated_accepted']:,} / {c['mutated_rejected']:,}"
        elif key=='firing':
            value=f"{c['firing_coincide_antecedent']:,} / {c['firing_strict_gap']:,}"
        elif key=='temporal':
            value=f"{c['temporal_points_R']:,} / {c['temporal_points_S']:,}"
        elif key=='versions':
            value=f"{c['version_chains']:,} / {c['version_violations_rejected']:,}"
        else:
            value=f'{c[key]:,}'
        lines.append(label+' & '+value+' '+chr(92)*2)
    header = r'\begin{tabular}{L{10.2cm}r}' + '\n' + r'\toprule' + '\n' + r'Check family & Cases \\' + '\n' + r'\midrule' + '\n'
    footer = r'\bottomrule' + '\n' + r'\end{tabular}' + '\n'
    args.output.with_name('validation-counts.tex').write_text(header+'\n'.join(lines)+'\n'+footer)
    import check_extensions
    args.output.with_name('ablation-table.tex').write_text(check_extensions.ablation_table_tex())
    print(json.dumps(result,indent=2))

if __name__=='__main__':
    main()
