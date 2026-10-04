"""Independent finite-model checks for paper revision 2; Python standard library."""
import itertools,json,random,sys
from pathlib import Path
SEED=20261003
rng=random.Random(SEED)
counts={}
def support(refs,x):
 seen={x};todo=[x]
 while todo:
  for y in refs.get(todo.pop(),set()):
   if y not in seen:seen.add(y);todo.append(y)
 return seen
def core(refs,X):return {x for x in X if support(refs,x)<=X}
def induced(refs,X):return {x:refs[x]&X for x in X}
def deps(refs,X,r):return {x for x in X if r in support(induced(refs,X),x)}
def sub(X):
 xs=sorted(X)
 return [{x for i,x in enumerate(xs) if k>>i&1} for k in range(1<<len(xs))]
# Exhaust all self-free reference graphs on at most four occurrences, max two targets.
graphs=views=cascades=intersections=0
for n in range(1,5):
 S=set(range(n)); choices=[list(filter(lambda v:len(v)<=2,sub(S-{x}))) for x in range(n)]
 for picks in itertools.product(*choices):
  refs=dict(enumerate(picks));graphs+=1;sets=sub(S);cores=[core(refs,X) for X in sets]
  for X,G in zip(sets,cores):
   views+=1;assert G<=X and core(refs,G)==G
   for Y,H in zip(sets,cores):
    assert core(refs,X&Y)==G&H;intersections+=1
    if X<=Y:assert G<=H
    if G==X and H==Y:assert core(refs,X|Y)==X|Y
   if G==X:
    for r in X:
     C=deps(refs,X,r);assert X-C==core(refs,X-{r});cascades+=1
     assert all(C<=R for R in sub(X) if r in R and core(refs,X-R)==X-R)
  # Check effective validity for every subset with randomly assigned finite intervals.
  valid={x:set(range(a,a+rng.randrange(1,5))) for x in S for a in [rng.randrange(5)]}
  for X in sets:
   for v in range(9):
    lhs=core(refs,{x for x in X if v in valid[x]})
    rhs={x for x in X if support(refs,x)<=X and all(v in valid[y] for y in support(refs,x))}
    assert lhs==rhs
counts.update(reference_graphs=graphs,row_subsets=views,intersection_checks=intersections,minimal_cascades=cascades)
# Structural correction on generated complete histories, including reference cycles and lineage.
corrections=0
for trial in range(2000):
 S=set(range(rng.randrange(2,9)));refs={x:set(rng.sample(sorted(S-{x}),rng.randrange(min(2,len(S)-1)+1))) for x in S}
 lineage={x for x in S if rng.randrange(4)==0};struct={x:set() if x in lineage else refs[x] for x in S}
 X=core(struct,{x for x in S if rng.randrange(4)})
 if not X:continue
 r=rng.choice(sorted(X));C=deps(refs,X,r);F={x for x in C-{r} if rng.randrange(4)==0};R=core(induced(struct,C),C-F)
 if r not in R:continue
 sigma={x:100+x for x in R};f=lambda y:sigma.get(y,y)
 clone={sigma[x]:{f(y) for y in refs[x]} for x in R}
 # Admissible root patch: choose up to two post-operation endpoints; prohibit direct self.
 newlive=(X-C)|set(sigma.values());candidates=sorted(newlive-{sigma[r]})
 subject = min(refs[r]) if len(refs[r]) == 2 else None
 clone[sigma[r]] = ({f(subject)} if subject is not None else set())
 if candidates and rng.randrange(2): clone[sigma[r]].add(rng.choice(candidates))
 ell=1000;newrefs={**refs,**clone,ell:{sigma[r],r}};newlin=lineage|{sigma[x] for x in R&lineage}|{ell}
 newstruct={x:set() if x in newlin else ys for x,ys in newrefs.items()};Xp=newlive|{ell}
 assert core(newstruct,Xp)==Xp
 assert all(ys<=set(newrefs) for ys in newrefs.values())
 for x in R-{r}:
  for y in R:assert (y in refs[x])==(sigma[y] in clone[sigma[x]])
 corrections+=1
counts['admissible_corrections']=corrections
# Encoding and decoding carry identities, duplicate annotation/membership occurrences, and clocks.
def encode(M):
 rows={}
 for x,c in M.items():
  if c['kind']=='v':rows['h:'+x]={'triple':('n:'+x,'rdf:type','mg:Vertex'),'life':c['life'],'valid':c['valid']}
  elif c['kind']=='e':
   rows['h:'+x]={'triple':('n:'+x,'mg:Edge',c['label']),'life':c['life'],'valid':c['valid']}
   for role in ['src','tgt']:
    for y in c[role]:rows[role+':'+x+':'+y]={'triple':('h:'+x,'mg:'+role,'h:'+y),'life':c['life'],'valid':c['valid']}
  elif c['kind']=='m':rows[x]={'triple':('h:'+c['mem'],'sys:inGraph','h:'+c['con']),'life':c['life'],'valid':c['valid']}
  else:rows[x]={'triple':('h:'+c['bear'],c['key'],('literal',c['value'])),'life':c['life'],'valid':c['valid']}
 return rows
def decode(rows):
 M={};anchors={}
 for eid,c in rows.items():
  a,p,b=c['triple'];clock={k:c[k] for k in ['life','valid']}
  if p in ['rdf:type','mg:Edge']:
   x=eid.removeprefix('h:');anchors[eid]=x;M[x]={'kind':'v',**clock} if p=='rdf:type' else {'kind':'e','label':b,'src':set(),'tgt':set(),**clock}
 for eid,c in rows.items():
  a,p,b=c['triple'];clock={k:c[k] for k in ['life','valid']}
  if p in ['mg:src','mg:tgt']:M[anchors[a]][p.split(':')[1]].add(anchors[b])
  elif p=='sys:inGraph':M[eid]={'kind':'m','mem':anchors[a],'con':anchors[b],**clock}
  elif p not in ['rdf:type','mg:Edge']:M[eid]={'kind':'a','bear':anchors[a],'key':p,'value':b[1],**clock}
 return M
def rowrefs(rows):return {eid:{x for x in [c['triple'][0],c['triple'][2]] if isinstance(x,str) and x.startswith('h:')} for eid,c in rows.items()}
def alive(c,t,v):return c['life'][0]<=t<c['life'][1] and c['valid'][0]<=v<c['valid'][1]
roundtrips=snapshots=0
for trial in range(200):
 M={x:{'kind':'v','life':(1,6),'valid':(0,5)} for x in ['v0','v1']};atoms=['v0','v1','e0','e1','e2']
 for x in atoms[2:]:
  other=[y for y in atoms if y!=x];M[x]={'kind':'e','label':rng.choice(['p','mg:Vertex']),'src':set(rng.sample(other,rng.randrange(1,3))),'tgt':set(rng.sample(other,rng.randrange(1,3))),'life':(1,5),'valid':(0,4)}
 for i in range(3):
  mem,con=rng.sample(atoms,2);M['m'+str(i)]={'kind':'m','mem':mem,'con':con,'life':(2,4),'valid':(1,3)}
 for i in range(3):M['a'+str(i)]={'kind':'a','bear':rng.choice(atoms),'key':'confidence','value':80,'life':(2,4),'valid':(1,3)}
 rows=encode(M);assert decode(rows)==M;assert encode(decode(rows))==rows;roundtrips+=1
 for t in range(7):
  for v in range(6):
   snap={x:c for x,c in rows.items() if alive(c,t,v)};expected={x:c for x,c in M.items() if alive(c,t,v)}
   assert core(rowrefs(rows),set(snap))==set(snap)
   assert decode(snap)==expected;snapshots+=1
counts.update(metagraph_roundtrips=roundtrips,bitemporal_snapshots=snapshots)
# Hop-layer recurrence versus enumeration, with random two-state product transitions.
journeys=0
for trial in range(1000):
 states=list(range(8));adj={x:[] for x in states}
 for x in states:
  for _ in range(rng.randrange(4)):
   a=rng.randrange(5);adj[x].append((rng.choice(states),a,a+rng.randrange(1,5)))
 K=rng.randrange(1,6);tau0=rng.randrange(3);B={0:tau0}
 for _ in range(K):
  nxt=dict(B)
  for x,tau in B.items():
   for y,a,b in adj[x]:
    if tau<b:nxt[y]=min(nxt.get(y,float('inf')),max(tau,a))
  B=nxt
 brute={0:tau0};todo=[(0,tau0,0)]
 while todo:
  x,tau,h=todo.pop()
  if h==K:continue
  for y,a,b in adj[x]:
   if tau<b:
    at=max(tau,a);brute[y]=min(brute.get(y,float('inf')),at);todo.append((y,at,h+1))
 assert B==brute;journeys+=1
counts['bounded_journeys']=journeys
# Original failure witnesses are retained as excluded cases, not assumed away silently.
def complete(refs):return all(targets<=set(refs) for targets in refs.values())
assert not complete({0:{1}})
# Reject duplicated general roles and role clock mismatch in encoding normal form.
def coherent_unique_roles(rows):
 seen=set()
 for eid,c in rows.items():
  a,p,b=c['triple']
  if p not in ['mg:src','mg:tgt']:continue
  shape=(a,p,b)
  if shape in seen or a not in rows or b not in rows:return False
  if any(c[k]!=rows[a][k] for k in ['life','valid']):return False
  seen.add(shape)
 return True
roles=[k for k,c in rows.items() if c['triple'][1] in ['mg:src','mg:tgt']]
assert coherent_unique_roles(rows)
bad=dict(rows);bad['duplicate-role']=dict(rows[roles[0]]);assert not coherent_unique_roles(bad)
bad=dict(rows);bad[roles[0]]={**bad[roles[0]],'valid':(1,2)};assert not coherent_unique_roles(bad)
cycle={0:{1},1:{0}};assert support(cycle,0)=={0,1}
counts['excluded_boundary_witnesses']=4
print(json.dumps({'seed':SEED,'status':'PASS','checks':counts},indent=2))
if len(sys.argv)>1:Path(sys.argv[1]).write_text(json.dumps({'seed':SEED,'status':'PASS','checks':counts},indent=2)+'\n')
