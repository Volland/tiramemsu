"""Named graphs on tiramemsu's triple schema: the storage cost of memberships and the
cost of `GRAPH <g>` / `GRAPH ?g` patterns (change add-named-graphs, task 7.2).

Same data formulas as ../engine-comparison/bench.py, SQLite only (no DuckDB needed).
A membership is a row `(s = statement eid, p = sys:inGraph, o = graph)`.
The SQL is the shape tm-exec generates for a graph selector: a join of the pattern with
`triple AS m` under the same live predicate.

Usage: python3 bench.py N   (N nodes; statements ~= 11*N; live statements ~= 7*N)
Database files go to $BENCH_DIR (default: the system temp dir); results JSON goes next to this script.
"""
import os, sys, time, json, sqlite3, statistics, tempfile

N = int(sys.argv[1]) if len(sys.argv) > 1 else 100_000
HERE = os.path.dirname(os.path.abspath(__file__))
SP = os.environ.get("BENCH_DIR", tempfile.gettempdir())
TY, NAME, CEO, KNOWS = 16, 32, 48, 64
C3, C7, C5 = 48, 112, 80
IG = 96  # sys:inGraph
NGRAPHS = 100
GRAPH0 = 1_000_000_000 * 16  # graph node ids, far from data ids
SMALL, LARGE = GRAPH0 + 1000 * 16, GRAPH0 + 1001 * 16
res = {"N": N, "sqlite": sqlite3.sqlite_version}

SCHEMA = """
PRAGMA journal_mode=WAL;
CREATE TABLE triple (eid INTEGER PRIMARY KEY, s INTEGER NOT NULL, p INTEGER NOT NULL, o INTEGER NOT NULL,
  t_add INTEGER NOT NULL, t_ret INTEGER, v_from INTEGER, v_to INTEGER, ret_kind INTEGER) STRICT;
"""
INDEXES = """
CREATE INDEX live_spo ON triple(s, p, o, t_ret, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX live_pos ON triple(p, o, s, t_ret, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX live_osp ON triple(o, s, p, t_ret, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX hist_spo ON triple(s, p, o, t_add DESC, t_ret, v_from, v_to);
CREATE INDEX hist_pos ON triple(p, o, s, t_add DESC, t_ret, v_from, v_to);
CREATE INDEX hist_osp ON triple(o, s, p, t_add DESC, t_ret, v_from, v_to);
CREATE INDEX valid_p ON triple(p, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX log_add ON triple(t_add);
CREATE INDEX log_ret ON triple(t_ret, ret_kind) WHERE t_ret IS NOT NULL;
"""
DATA = f"""
WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{N})
INSERT INTO triple(s,p,o,t_add,t_ret,ret_kind)
SELECT i*16, {TY}, CASE WHEN i%10=0 THEN {C3} ELSE {C7} END, 1, NULL, NULL FROM n
UNION ALL SELECT i*16, {NAME}, (10000000+i)*16+10, 1, NULL, NULL FROM n
UNION ALL SELECT i*16, {KNOWS}, ((i*7919+k)%{N}+1)*16, 1, NULL, NULL FROM n,
  (SELECT 1 k UNION SELECT 2 UNION SELECT 3 UNION SELECT 4 UNION SELECT 5)
UNION ALL SELECT i*16, {CEO}, {C5}, 1, NULL, NULL FROM n WHERE i%2000=0
UNION ALL SELECT i*16, {NAME}, (20000000+i*4+k)*16+10, 1, 2, 2 FROM n,
  (SELECT 0 k UNION SELECT 1 UNION SELECT 2 UNION SELECT 3);
"""


def timed(fn, reps):
    xs = []
    for _ in range(reps):
        s = time.perf_counter()
        out = fn()
        xs.append(time.perf_counter() - s)
    return statistics.median(xs) * 1e3, out


def size(path):
    return sum(os.path.getsize(path + s) for s in ("", "-wal") if os.path.exists(path + s))


path = os.path.join(SP, f"graphs-{N}.db")
for suf in ("", "-wal", "-shm"):
    if os.path.exists(path + suf):
        os.remove(path + suf)
c = sqlite3.connect(path, isolation_level=None)
c.execute("PRAGMA synchronous=OFF")
c.executescript(SCHEMA)
t0 = time.perf_counter()
c.executescript("BEGIN;" + DATA + "COMMIT;")
c.executescript(INDEXES + "ANALYZE;")
c.execute("PRAGMA wal_checkpoint(TRUNCATE)")
res["load_s"] = time.perf_counter() - t0
res["rows_before"] = c.execute("SELECT count(*) FROM triple").fetchone()[0]
res["live_before"] = c.execute("SELECT count(*) FROM triple WHERE t_ret IS NULL").fetchone()[0]
res["bytes_before"] = size(path)

# every live statement gets one membership, in one of NGRAPHS graphs
t0 = time.perf_counter()
c.execute("BEGIN")
c.execute(
    f"INSERT INTO triple(s,p,o,t_add) SELECT eid, {IG}, {GRAPH0}+(eid%{NGRAPHS})*16, 2 "
    f"FROM triple WHERE t_ret IS NULL AND p<>{IG}"
)
c.execute("COMMIT")
c.executescript("ANALYZE;")
c.execute("PRAGMA wal_checkpoint(TRUNCATE)")
res["membership_load_s"] = time.perf_counter() - t0
res["rows_after"] = c.execute("SELECT count(*) FROM triple").fetchone()[0]
res["bytes_after"] = size(path)
res["storage_ratio"] = res["bytes_after"] / res["bytes_before"]
res["row_ratio"] = res["rows_after"] / res["rows_before"]
res["live_ratio"] = c.execute("SELECT count(*) FROM triple WHERE t_ret IS NULL").fetchone()[0] / res["live_before"]

# a 100-member graph and a graph holding every `knows` statement (5N members)
c.execute("BEGIN")
c.execute(
    f"INSERT INTO triple(s,p,o,t_add) SELECT eid, {IG}, {SMALL}, 3 FROM triple WHERE t_ret IS NULL AND p={KNOWS} ORDER BY eid LIMIT 100"
)
c.execute(
    f"INSERT INTO triple(s,p,o,t_add) SELECT eid, {IG}, {LARGE}, 3 FROM triple WHERE t_ret IS NULL AND p={KNOWS}"
)
c.execute("COMMIT")
c.executescript("ANALYZE;")
res["small_members"] = c.execute(f"SELECT count(*) FROM triple WHERE p={IG} AND o={SMALL} AND t_ret IS NULL").fetchone()[0]
res["large_members"] = c.execute(f"SELECT count(*) FROM triple WHERE p={IG} AND o={LARGE} AND t_ret IS NULL").fetchone()[0]

# the SQL of `GRAPH <g> { ?s <pred> ?o }` (membership joined under the live predicate)
GRAPH_Q = (
    "SELECT count(*) FROM triple t0, triple t1 WHERE t0.p=? AND t0.t_ret IS NULL "
    "AND t1.p=? AND t1.o=? AND t1.t_ret IS NULL AND t0.eid=t1.s"
)
BASE_Q = "SELECT count(*) FROM triple t0 WHERE t0.p=? AND t0.t_ret IS NULL"
ANY_Q = (  # GRAPH ?g over every statement
    "SELECT t1.o, count(*) FROM triple t0, triple t1 WHERE t1.p=? AND t1.t_ret IS NULL "
    "AND t0.eid=t1.s AND t0.t_ret IS NULL GROUP BY t1.o"
)
res["queries"] = {}


def q(name, sql, args, reps):
    ms, out = timed(lambda: c.execute(sql, args).fetchall(), reps)
    plan = [r[3] for r in c.execute("EXPLAIN QUERY PLAN " + sql, args)]
    res["queries"][name] = {"ms": round(ms, 3), "rows": out[0][0] if len(out) == 1 else len(out), "plan": plan}


# a `knows` statement is in one of 100 partition graphs and in LARGE; 100 of them are also in SMALL
q("no_graph_knows_count", BASE_Q, (KNOWS,), 5)
q("graph_small_knows", GRAPH_Q, (KNOWS, IG, SMALL), 20)
q("graph_partition_knows", GRAPH_Q, (KNOWS, IG, GRAPH0 + 7 * 16), 5)
q("graph_large_knows", GRAPH_Q, (KNOWS, IG, LARGE), 3)
q("graph_var_all", ANY_Q, (IG,), 1)
# the small graph and a pattern with many matches: the small side must drive the join
q("graph_small_type_person", GRAPH_Q, (TY, IG, SMALL), 20)

out = os.path.join(HERE, f"results-{N}.json")
json.dump(res, open(out, "w"), indent=1)
print(json.dumps({k: v for k, v in res.items() if k != "queries"}, indent=1))
for k, v in res["queries"].items():
    print(f"{k:38s} {v['ms']:10.3f} ms  rows={v['rows']}  {v['plan'][0][:70]}")
