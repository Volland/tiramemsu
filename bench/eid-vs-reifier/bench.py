"""What does a statement id (eid) cost and buy?  A = tiramemsu (eid rowid), B = quads-style
WITHOUT ROWID PK(s,p,o) with RDF 1.2 reifiers for annotations (oxilite's D14 model).
Live data only, 3 covering permutations each, so the eid is the only difference."""
import os, sqlite3, time, random, tempfile
N = 100_000
D = tempfile.mkdtemp()
TY, NAME, KNOWS, CONF, SRC, REIFIES = 16, 32, 64, 96, 112, 128
random.seed(1)


def t(fn, reps):
    s = time.perf_counter()
    for i in range(reps):
        r = fn(i)
    return (time.perf_counter() - s) / reps, r


def base_rows():
    for i in range(1, N + 1):
        yield (i * 16, TY, 112)
        yield (i * 16, NAME, (10_000_000 + i) * 16 + 10)
        for k in range(1, 6):
            yield (i * 16, KNOWS, ((i * 7919 + k) % N + 1) * 16)


# 10 % of knows edges get a confidence; 1 % of those confidences get a source (annotation of annotation)
edges = [(i * 16, KNOWS, ((i * 7919 + k) % N + 1) * 16) for i in range(1, N + 1) for k in range(1, 6)]
annotated = random.sample(edges, len(edges) // 10)
conf_of = {e: random.randint(0, 100) * 16 + 5 for e in annotated}          # INT-tagged 0..100
sourced = random.sample(annotated, len(annotated) // 100)

# ------------------------------------------------------------------ A: eid
a = sqlite3.connect(os.path.join(D, "a.db"), isolation_level=None)
a.executescript("""PRAGMA journal_mode=WAL; PRAGMA synchronous=OFF;
CREATE TABLE triple(eid INTEGER PRIMARY KEY, s INT NOT NULL, p INT NOT NULL, o INT NOT NULL) STRICT;""")
a.execute("BEGIN")
a.executemany("INSERT INTO triple(s,p,o) VALUES (?,?,?)", base_rows())
eid = {r[1:]: r[0] for r in a.execute("SELECT eid, s, p, o FROM triple WHERE p=?", (KNOWS,))}
ann_eid = {}
for e in annotated:
    cur = a.execute("INSERT INTO triple(s,p,o) VALUES (?,?,?)", (eid[e] * 16 + 3, CONF, conf_of[e]))  # s = STMT id
    ann_eid[e] = cur.lastrowid
for e in sourced:
    a.execute("INSERT INTO triple(s,p,o) VALUES (?,?,?)", (ann_eid[e] * 16 + 3, SRC, 777 * 16))
a.execute("COMMIT")
a.executescript("""CREATE INDEX spo ON triple(s,p,o); CREATE INDEX pos ON triple(p,o,s); CREATE INDEX osp ON triple(o,s,p);
ANALYZE; VACUUM;""")

# ------------------------------------------------------------------ B: quads + reifiers
b = sqlite3.connect(os.path.join(D, "b.db"), isolation_level=None)
b.executescript("""PRAGMA journal_mode=WAL; PRAGMA synchronous=OFF;
CREATE TABLE quad(s INT NOT NULL, p INT NOT NULL, o INT NOT NULL, PRIMARY KEY(s,p,o)) WITHOUT ROWID, STRICT;
CREATE TABLE triple_term(id INTEGER PRIMARY KEY, s INT NOT NULL, p INT NOT NULL, o INT NOT NULL, UNIQUE(s,p,o)) STRICT;""")
b.execute("BEGIN")
b.executemany("INSERT INTO quad VALUES (?,?,?)", base_rows())
rid = 50_000_000
reif = {}
for e in annotated:                       # _:r rdf:reifies <<( s p o )>> ; :conf v
    tt = b.execute("INSERT INTO triple_term(s,p,o) VALUES (?,?,?)", e).lastrowid
    r = rid * 16 + 2; rid += 1
    b.execute("INSERT INTO quad VALUES (?,?,?)", (r, REIFIES, tt * 16 + 13))
    b.execute("INSERT INTO quad VALUES (?,?,?)", (r, CONF, conf_of[e]))
    reif[e] = r
for e in sourced:                         # annotate the confidence triple: needs its own reifier
    r, v = reif[e], conf_of[e]
    tt = b.execute("INSERT INTO triple_term(s,p,o) VALUES (?,?,?)", (r, CONF, v)).lastrowid
    r2 = rid * 16 + 2; rid += 1
    b.execute("INSERT INTO quad VALUES (?,?,?)", (r2, REIFIES, tt * 16 + 13))
    b.execute("INSERT INTO quad VALUES (?,?,?)", (r2, SRC, 777 * 16))
b.execute("COMMIT")
b.executescript("CREATE INDEX pos ON quad(p,o,s); CREATE INDEX osp ON quad(o,s,p); ANALYZE; VACUUM;")

sz = lambda n: os.path.getsize(os.path.join(D, n)) / 1e6
print(f"rows A={a.execute('select count(*) from triple').fetchone()[0]:,} B={b.execute('select count(*) from quad').fetchone()[0]:,} (+{b.execute('select count(*) from triple_term').fetchone()[0]:,} triple terms)")
print(f"size MB   A(eid)={sz('a.db'):.1f}   B(quads+reifiers)={sz('b.db'):.1f}")

xs = [e[0] for e in random.sample(annotated, 2000)]

Q = {
 "plain point (node's names)": (
   "SELECT o FROM triple WHERE s=? AND p=32",
   "SELECT o FROM quad WHERE s=? AND p=32", lambda i: (xs[i],), 2000),
 "edges of a node + confidence": (
   "SELECT t.o, c.o FROM triple t LEFT JOIN triple c ON c.s=t.eid*16+3 AND c.p=96 WHERE t.s=? AND t.p=64",
   "SELECT q.o, c.o FROM quad q LEFT JOIN triple_term tt ON tt.s=q.s AND tt.p=q.p AND tt.o=q.o "
   "LEFT JOIN quad r ON r.p=128 AND r.o=tt.id*16+13 LEFT JOIN quad c ON c.s=r.s AND c.p=96 WHERE q.s=? AND q.p=64",
   lambda i: (xs[i],), 2000),
 "all edges with confidence >= 95": (
   "SELECT count(*) FROM triple c JOIN triple t ON t.eid=c.s>>4 WHERE c.p=96 AND c.o>=95*16+5",
   "SELECT count(*) FROM quad c JOIN quad r ON r.s=c.s AND r.p=128 JOIN triple_term tt ON tt.id=r.o>>4 "
   "JOIN quad q ON q.s=tt.s AND q.p=tt.p AND q.o=tt.o WHERE c.p=96 AND c.o>=95*16+5", lambda i: (), 20),
 "source of an edge's confidence (2 levels)": (
   "SELECT s2.o FROM triple t JOIN triple c ON c.s=t.eid*16+3 AND c.p=96 JOIN triple s2 ON s2.s=c.eid*16+3 AND s2.p=112 WHERE t.s=? AND t.p=64",
   "SELECT s2.o FROM quad q JOIN triple_term tt ON tt.s=q.s AND tt.p=q.p AND tt.o=q.o JOIN quad r ON r.p=128 AND r.o=tt.id*16+13 "
   "JOIN quad c ON c.s=r.s AND c.p=96 JOIN triple_term tt2 ON tt2.s=c.s AND tt2.p=c.p AND tt2.o=c.o "
   "JOIN quad r2 ON r2.p=128 AND r2.o=tt2.id*16+13 JOIN quad s2 ON s2.s=r2.s AND s2.p=112 WHERE q.s=? AND q.p=64",
   lambda i: (xs[i],), 2000),
 "2-hop, set semantics": (
   # A must dedup (s,p,o) over eids: the canonical-eid predicate from the M1 design
   "SELECT count(*) FROM triple a JOIN triple b ON b.s=a.o WHERE a.s=? AND a.p=64 AND b.p=32 "
   "AND a.eid=(SELECT min(x.eid) FROM triple x WHERE x.s=a.s AND x.p=a.p AND x.o=a.o) "
   "AND b.eid=(SELECT min(x.eid) FROM triple x WHERE x.s=b.s AND x.p=b.p AND x.o=b.o)",
   "SELECT count(*) FROM quad a JOIN quad b ON b.s=a.o WHERE a.s=? AND a.p=64 AND b.p=32", lambda i: (xs[i],), 2000),
 "2-hop, bag (no dedup)": (
   "SELECT count(*) FROM triple a JOIN triple b ON b.s=a.o WHERE a.s=? AND a.p=64 AND b.p=32",
   "SELECT count(*) FROM quad a JOIN quad b ON b.s=a.o WHERE a.s=? AND a.p=64 AND b.p=32", lambda i: (xs[i],), 2000),
}
print(f"{'query':44} {'A eid':>10} {'B reifier':>10}  ratio")
for name, (qa, qb, args, n) in Q.items():
    ta, ra = t(lambda i: a.execute(qa, args(i)).fetchall(), n)
    tb, rb = t(lambda i: b.execute(qb, args(i)).fetchall(), n)
    unit, k = ("ms", 1e3) if n < 100 else ("µs", 1e6)
    same = "" if (n < 100 and ra == rb) or n >= 100 else f"  MISMATCH {ra} {rb}"
    print(f"{name:44} {ta*k:8.1f}{unit} {tb*k:8.1f}{unit}  B/A={tb/ta:4.1f}{same}")

# write cost: add an edge + a confidence in one tx
def wa(i):
    a.execute("BEGIN"); e = a.execute("INSERT INTO triple(s,p,o) VALUES (?,?,?)", (16*(i+1), KNOWS, 16*(i+7))).lastrowid
    a.execute("INSERT INTO triple(s,p,o) VALUES (?,?,?)", (e*16+3, CONF, 50*16+5)); a.execute("COMMIT")
def wb(i):
    b.execute("BEGIN"); s, o = 16*(i+1), 16*(i+900_001)
    b.execute("INSERT OR IGNORE INTO quad VALUES (?,?,?)", (s, KNOWS, o))
    tt = b.execute("INSERT INTO triple_term(s,p,o) VALUES (?,?,?)", (s, KNOWS, o)).lastrowid
    r = (60_000_000 + i) * 16 + 2
    b.execute("INSERT INTO quad VALUES (?,?,?)", (r, REIFIES, tt*16+13)); b.execute("INSERT INTO quad VALUES (?,?,?)", (r, CONF, 50*16+5))
    b.execute("COMMIT")
ta, _ = t(wa, 2000); tb, _ = t(wb, 2000)
print(f"{'write: edge + confidence (1 tx)':44} {ta*1e6:8.1f}µs {tb*1e6:8.1f}µs  B/A={tb/ta:4.1f}")
