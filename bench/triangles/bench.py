import sqlite3, time, random, bisect
random.seed(3)
K=64
def build(edges, name):
    c=sqlite3.connect(":memory:")
    c.executescript("""CREATE TABLE triple(eid INTEGER PRIMARY KEY,s INT NOT NULL,p INT NOT NULL,o INT NOT NULL,t_add INT NOT NULL,t_ret INT,v_from INT,v_to INT,ret_kind INT) STRICT;""")
    c.executemany("INSERT INTO triple(s,p,o,t_add) VALUES(?,?,?,1)", [(a*16,K,b*16) for a,b in edges])
    c.executescript("""CREATE INDEX live_spo ON triple(s,p,o,t_ret,v_from,v_to) WHERE t_ret IS NULL;
    CREATE INDEX live_pos ON triple(p,o,s,t_ret,v_from,v_to) WHERE t_ret IS NULL;
    CREATE INDEX live_osp ON triple(o,s,p,t_ret,v_from,v_to) WHERE t_ret IS NULL; ANALYZE;""")
    return c
Q="""SELECT count(*) FROM triple a JOIN triple b ON b.s=a.o AND b.p=? AND b.t_ret IS NULL
 JOIN triple c ON c.s=b.o AND c.p=? AND c.t_ret IS NULL AND c.o=a.s
 WHERE a.p=? AND a.t_ret IS NULL"""

def wco(edges):
    """Generic-join / leapfrog style: bind x, then y in out(x), then z in out(y) ∩ in(x), iterating the smaller side."""
    out={}; inn={}
    for a,b in edges:
        out.setdefault(a,set()).add(b); inn.setdefault(b,set()).add(a)
    n=0
    for x,ys in out.items():
        ix=inn.get(x)
        if not ix: continue
        for y in ys:
            oy=out.get(y)
            if not oy: continue
            n+=len(oy&ix) if len(oy)>len(ix) else len(ix&oy)   # C-speed set intersection, cost ~ min(|oy|,|ix|)
    return n
def run(name, edges):
    edges=list(set(edges))
    c=build(edges,name)
    t=time.perf_counter(); r=c.execute(Q,(K,K,K)).fetchone()[0]; ts=time.perf_counter()-t
    t=time.perf_counter(); r2=wco(edges); tl=time.perf_counter()-t
    print(f"{name:40} edges={len(edges):>8,} triangles={r:>10,} sqlite(nested loop+index)={ts*1e3:9.1f} ms  intersection join (python)={tl*1e3:8.1f} ms  ratio={ts/tl:6.1f}x same={r==r2}")
N=100_000
run("uniform, out-degree 5", [(i,(i*7919+k)%N+1) for i in range(1,N+1) for k in range(1,6)])
hubs=range(1,301); spokes=range(1001,2501)
E=[(h,s) for h in hubs for s in spokes]+[(s,h) for h in hubs for s in spokes]
E+=[(random.randint(1,50000),random.randint(1,50000)) for _ in range(200000)]
run("skewed: 300 hubs x 1500 spokes", E)
for n in (150,300,600):
    A=list(range(1,n+1));B=list(range(10001,10001+n));C=list(range(20001,20001+n))
    E=[(a,b) for a in A for b in B]+[(b,c) for b in B for c in C]+[(c,a) for c in C for a in random.sample(A,3)]
    run(f"adversarial: 3 layers of {n}, 3 closers each", E)
