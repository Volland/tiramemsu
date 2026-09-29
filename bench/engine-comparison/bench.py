"""SQLite vs DuckDB on tiramemsu's triple schema and workload.

Same data (deterministic formulas), same queries, prepared statements, one process.
Usage: python bench.py N   (N nodes; statements ~= 11*N)
Database files go to $BENCH_DIR (default: the system temp dir); results JSON goes next to this script.
"""
import os, sys, time, random, sqlite3, json, statistics, tempfile
import duckdb

N = int(sys.argv[1]) if len(sys.argv) > 1 else 100_000
HERE = os.path.dirname(os.path.abspath(__file__))
# database files are large (1.7 GB at N=1M): keep them out of the repo
SP = os.environ.get("BENCH_DIR", tempfile.gettempdir())
TY, NAME, CEO, KNOWS = 16, 32, 48, 64
C3, C7, C5 = 48, 112, 80
random.seed(7)
res = {"N": N}


def t(fn, reps=1):
    s = time.perf_counter()
    for _ in range(reps):
        out = fn()
    return (time.perf_counter() - s), out


# ---------------------------------------------------------------- data (SQL, same formulas)
SQLITE_DATA = f"""
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
DUCK_DATA = f"""
INSERT INTO triple
SELECT row_number() OVER () AS eid, * FROM (
  SELECT i*16 s, {TY} p, CASE WHEN i%10=0 THEN {C3} ELSE {C7} END o, 1 t_add, NULL::BIGINT t_ret, NULL::BIGINT v_from, NULL::BIGINT v_to, NULL::INTEGER ret_kind FROM range(1,{N}+1) r(i)
  UNION ALL SELECT i*16, {NAME}, (10000000+i)*16+10, 1, NULL, NULL, NULL, NULL FROM range(1,{N}+1) r(i)
  UNION ALL SELECT i*16, {KNOWS}, ((i*7919+k)%{N}+1)*16, 1, NULL, NULL, NULL, NULL FROM range(1,{N}+1) r(i), range(1,6) q(k)
  UNION ALL SELECT i*16, {CEO}, {C5}, 1, NULL, NULL, NULL, NULL FROM range(1,{N}+1) r(i) WHERE i%2000=0
  UNION ALL SELECT i*16, {NAME}, (20000000+i*4+k)*16+10, 1, 2, NULL, NULL, 2 FROM range(1,{N}+1) r(i), range(0,4) q(k)
  ORDER BY 1, 2, 3
);
"""

SQLITE_SCHEMA = """
PRAGMA journal_mode=WAL;
CREATE TABLE triple (eid INTEGER PRIMARY KEY, s INTEGER NOT NULL, p INTEGER NOT NULL, o INTEGER NOT NULL,
  t_add INTEGER NOT NULL, t_ret INTEGER, v_from INTEGER, v_to INTEGER, ret_kind INTEGER) STRICT;
"""
SQLITE_INDEXES = """
CREATE INDEX live_spo ON triple(s, p, o, t_ret, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX live_pos ON triple(p, o, s, t_ret, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX live_osp ON triple(o, s, p, t_ret, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX hist_spo ON triple(s, p, o, t_add DESC, t_ret, v_from, v_to);
CREATE INDEX hist_pos ON triple(p, o, s, t_add DESC, t_ret, v_from, v_to);
CREATE INDEX hist_osp ON triple(o, s, p, t_add DESC, t_ret, v_from, v_to);
CREATE INDEX valid_p ON triple(p, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX log_add ON triple(t_add);
CREATE INDEX log_ret ON triple(t_ret, ret_kind) WHERE t_ret IS NOT NULL;
ANALYZE;
"""
# DuckDB: no partial indexes, no covering indexes; ART on the lookup prefixes + PK for eid
DUCK_SCHEMA = """
CREATE TABLE triple (eid BIGINT PRIMARY KEY, s BIGINT NOT NULL, p BIGINT NOT NULL, o BIGINT NOT NULL,
  t_add BIGINT NOT NULL, t_ret BIGINT, v_from BIGINT, v_to BIGINT, ret_kind INTEGER);
"""
DUCK_INDEXES = """
CREATE INDEX i_s ON triple(s);
CREATE INDEX i_o ON triple(o);
CREATE INDEX i_po ON triple(p, o);
"""


def open_sqlite(path, sync):
    for suf in ("", "-wal", "-shm"):
        if os.path.exists(path + suf):
            os.remove(path + suf)
    c = sqlite3.connect(path, isolation_level=None, check_same_thread=False)
    c.execute(f"PRAGMA synchronous={sync}")
    return c


def open_duck(path, indexes=True):
    for suf in ("", ".wal"):
        if os.path.exists(path + suf):
            os.remove(path + suf)
    return duckdb.connect(path)


def size(path, extra=("-wal",)):
    return sum(os.path.getsize(path + s) for s in ("",) + tuple(extra) if os.path.exists(path + s))


# ---------------------------------------------------------------- workloads
starts = [random.randint(1, N) * 16 for _ in range(2000)]


def workloads(name, cur_fn, q):
    """q: dict of prepared SQL strings (engine dialect)."""
    r = {}
    ex = cur_fn

    # point lookups, Now
    d, _ = t(lambda: [ex(q["point_now"], (s, NAME)) for s in starts])
    r["point_now_us"] = d / len(starts) * 1e6
    # point lookups, asOf(1): old names visible
    d, _ = t(lambda: [ex(q["point_asof"], (s, NAME, 1, 1)) for s in starts])
    r["point_asof_us"] = d / len(starts) * 1e6
    # 2-hop from bound start: ?x knows ?y . ?y name ?n
    d, _ = t(lambda: [ex(q["two_hop"], (s, KNOWS, NAME)) for s in starts[:500]])
    r["two_hop_us"] = d / 500 * 1e6
    # skewed 4-pattern BGP (rare pattern must go first)
    d, rows = t(lambda: ex(q["bgp4"], (TY, C7, KNOWS, CEO, C5, NAME)), reps=5)
    r["bgp4_ms"] = d / 5 * 1e3
    r["bgp4_rows"] = rows[0][0]
    # one BFS layer: out-neighbours of 256 frontier nodes, batched
    frontier = starts[:256]
    d, rows = t(lambda: ex(q["bfs_layer"].format(ids=",".join(map(str, frontier))), (KNOWS,)), reps=20)
    r["bfs_layer_256_ms"] = d / 20 * 1e3
    # 3-hop reachability via recursive CTE from one node (engine-native)
    d, rows = t(lambda: ex(q["reach3"], (starts[0], KNOWS, KNOWS)), reps=5)
    r["reach3_ms"] = d / 5 * 1e3
    r["reach3_rows"] = rows[0][0]
    # analytics: live statements per predicate, and whole-history aggregate asOf(1)
    d, _ = t(lambda: ex(q["agg_live"], ()), reps=3)
    r["agg_live_ms"] = d / 3 * 1e3
    d, _ = t(lambda: ex(q["agg_hist"], (1, 1)), reps=3)
    r["agg_hist_ms"] = d / 3 * 1e3
    return r


SQL_Q = {
    "point_now": "SELECT o, eid FROM triple WHERE s=? AND p=? AND t_ret IS NULL",
    "point_asof": "SELECT o, eid FROM triple WHERE s=? AND p=? AND t_add<=? AND (t_ret IS NULL OR t_ret>?)",
    "two_hop": "SELECT b.o FROM triple a JOIN triple b ON b.s=a.o WHERE a.s=? AND a.p=? AND a.t_ret IS NULL AND b.p=? AND b.t_ret IS NULL",
    "bgp4": "SELECT count(*) FROM triple t1, triple t2, triple t3, triple t4 WHERE t1.p=? AND t1.o=? AND t1.t_ret IS NULL "
            "AND t2.s=t1.s AND t2.p=? AND t2.t_ret IS NULL AND t3.s=t2.o AND t3.p=? AND t3.o=? AND t3.t_ret IS NULL "
            "AND t4.s=t3.s AND t4.p=? AND t4.t_ret IS NULL",
    "bfs_layer": "SELECT s, eid, o FROM triple WHERE s IN ({ids}) AND p=? AND t_ret IS NULL",
    "reach3": "WITH RECURSIVE r(n, d) AS (SELECT ?, 0 UNION SELECT t.o, r.d+1 FROM r JOIN triple t ON t.s=r.n "
              "WHERE t.p=? AND t.t_ret IS NULL AND r.d<3) SELECT count(DISTINCT n) FROM r",
    "agg_live": "SELECT p, count(*), count(DISTINCT s) FROM triple WHERE t_ret IS NULL GROUP BY p",
    "agg_hist": "SELECT p, count(*) FROM triple WHERE t_add<=? AND (t_ret IS NULL OR t_ret>?) GROUP BY p",
}
# reach3 in sqlite references ? twice in different places: KNOWS used once; fix param count per engine
SQL_Q["reach3"] = SQL_Q["reach3"].replace("WHERE t.p=?", "WHERE t.p=?2").replace("SELECT ?, 0", "SELECT ?1, 0")


def sqlite_suite(sync):
    path = os.path.join(SP, f"bench_{N}.sqlite")
    c = open_sqlite(path, sync)
    c.executescript(SQLITE_SCHEMA)
    d_load, _ = t(lambda: (c.execute("BEGIN"), c.execute(SQLITE_DATA), c.execute("COMMIT")))
    d_idx, _ = t(lambda: c.executescript(SQLITE_INDEXES))
    c.execute("PRAGMA wal_checkpoint(TRUNCATE)")
    r = {"load_s": d_load, "index_s": d_idx, "size_mb": size(path) / 1e6,
         "rows": c.execute("SELECT count(*) FROM triple").fetchone()[0]}

    def ex(sql, params):
        if sql.startswith("WITH RECURSIVE"):
            params = (params[0], params[1])
        return c.execute(sql, params).fetchall()

    r.update(workloads("sqlite", ex, SQL_Q))
    r.update(small_tx_sqlite(c))
    r["size_after_tx_mb"] = size(path) / 1e6
    c.close()
    return r


def small_tx_sqlite(c):
    """Agent-memory writes: idempotent assert (read-before-write) and retract, one tx each."""
    next_eid = c.execute("SELECT max(eid) FROM triple").fetchone()[0] + 1
    tx = 3
    n = 2000
    s0 = time.perf_counter()
    for k in range(n):
        s, o = random.randint(1, N) * 16, (30000000 + k) * 16 + 10
        c.execute("BEGIN IMMEDIATE")
        hit = c.execute("SELECT eid FROM triple WHERE s=? AND p=? AND o=? AND t_ret IS NULL", (s, NAME, o)).fetchone()
        if not hit:
            c.execute("INSERT INTO triple(eid,s,p,o,t_add) VALUES (?,?,?,?,?)", (next_eid, s, NAME, o, tx))
            next_eid += 1
        c.execute("COMMIT")
        tx += 1
    d_assert = time.perf_counter() - s0
    live = [r[0] for r in c.execute("SELECT eid FROM triple WHERE p=? AND t_ret IS NULL LIMIT 1000", (KNOWS,))]
    s0 = time.perf_counter()
    for eid in live:
        c.execute("BEGIN IMMEDIATE")
        c.execute("UPDATE triple SET t_ret=?, ret_kind=0 WHERE eid=? AND t_ret IS NULL", (tx, eid))
        c.execute("COMMIT")
        tx += 1
    d_ret = time.perf_counter() - s0
    # speculative with: SAVEPOINT + ROLLBACK TO
    try:
        c.execute("BEGIN IMMEDIATE"); c.execute("SAVEPOINT spec")
        c.execute("INSERT INTO triple(eid,s,p,o,t_add) VALUES (?,?,?,?,?)", (next_eid, 16, NAME, 26, tx))
        c.execute("ROLLBACK TO spec"); c.execute("RELEASE spec"); c.execute("COMMIT")
        sp = "yes"
    except Exception as e:
        sp = f"no: {e}"
    return {"assert_tx_us": d_assert / n * 1e6, "retract_tx_us": d_ret / len(live) * 1e6, "savepoint": sp}


def duck_suite():
    path = os.path.join(SP, f"bench_{N}.duckdb")
    c = open_duck(path)
    c.execute(DUCK_SCHEMA)
    d_load, _ = t(lambda: c.execute(DUCK_DATA))
    d_idx, _ = t(lambda: c.execute(DUCK_INDEXES))
    c.execute("CHECKPOINT")
    r = {"load_s": d_load, "index_s": d_idx, "size_mb": size(path, (".wal",)) / 1e6,
         "rows": c.execute("SELECT count(*) FROM triple").fetchone()[0]}
    q = {k: v.replace("?1", "$1").replace("?2", "$2") for k, v in SQL_Q.items()}

    def ex(sql, params):
        if sql.startswith("WITH RECURSIVE"):
            params = (params[0], params[1])
        return c.execute(sql, params).fetchall()

    r.update(workloads("duckdb", ex, q))
    # does DuckDB use the ART index for the point lookup?
    plan = c.execute("EXPLAIN " + SQL_Q["point_now"].replace("?", "16", 1).replace("?", "32"), ()).fetchall()
    r["point_plan_uses_index"] = "INDEX_SCAN" in str(plan).upper() or "Index Scan" in str(plan)
    r.update(small_tx_duck(c))
    c.execute("CHECKPOINT")
    r["size_after_tx_mb"] = size(path, (".wal",)) / 1e6
    try:
        c.execute("CREATE TRIGGER x BEFORE DELETE ON triple BEGIN SELECT 1; END")
        r["triggers"] = "yes"
    except Exception as e:
        r["triggers"] = f"no: {str(e).splitlines()[0][:80]}"
    try:
        c.execute("CREATE INDEX pi ON triple(s) WHERE t_ret IS NULL")
        r["partial_index"] = "yes"
    except Exception as e:
        r["partial_index"] = f"no: {str(e).splitlines()[0][:80]}"
    c.close()
    return r


def small_tx_duck(c):
    next_eid = c.execute("SELECT max(eid) FROM triple").fetchone()[0] + 1
    tx = 3
    n = 2000
    s0 = time.perf_counter()
    for k in range(n):
        s, o = random.randint(1, N) * 16, (30000000 + k) * 16 + 10
        c.execute("BEGIN TRANSACTION")
        hit = c.execute("SELECT eid FROM triple WHERE s=? AND p=? AND o=? AND t_ret IS NULL", (s, NAME, o)).fetchone()
        if not hit:
            c.execute("INSERT INTO triple(eid,s,p,o,t_add) VALUES (?,?,?,?,?)", (next_eid, s, NAME, o, tx))
            next_eid += 1
        c.execute("COMMIT")
        tx += 1
    d_assert = time.perf_counter() - s0
    live = [r[0] for r in c.execute("SELECT eid FROM triple WHERE p=? AND t_ret IS NULL LIMIT 1000", (KNOWS,)).fetchall()]
    s0 = time.perf_counter()
    for eid in live:
        c.execute("BEGIN TRANSACTION")
        c.execute("UPDATE triple SET t_ret=?, ret_kind=0 WHERE eid=? AND t_ret IS NULL", (tx, eid))
        c.execute("COMMIT")
        tx += 1
    d_ret = time.perf_counter() - s0
    try:
        c.execute("BEGIN TRANSACTION"); c.execute("SAVEPOINT spec")
        c.execute("ROLLBACK TO spec"); c.execute("COMMIT")
        sp = "yes"
    except Exception as e:
        c.execute("ROLLBACK")
        sp = f"no: {str(e).splitlines()[0][:80]}"
    return {"assert_tx_us": d_assert / n * 1e6, "retract_tx_us": d_ret / len(live) * 1e6, "savepoint": sp}


if __name__ == "__main__":
    res["sqlite_NORMAL"] = sqlite_suite("NORMAL")
    res["sqlite_FULL"] = {k: v for k, v in sqlite_suite("FULL").items()
                          if k in ("assert_tx_us", "retract_tx_us")}
    res["duckdb"] = duck_suite()
    res["versions"] = {"sqlite": sqlite3.sqlite_version, "duckdb": duckdb.__version__}
    print(json.dumps(res, indent=1, default=str))
    with open(os.path.join(HERE, f"results-{N}.json"), "w") as f:
        json.dump(res, f, indent=1, default=str)
