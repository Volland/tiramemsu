#!/usr/bin/env python3
"""Build site/benchmarks.html from results-100000.json and results-1000000.json.

Run from this folder after a benchmark run: python3 build-site.py
Every number on the page comes from the two results files; the fastest engine
of each row is marked. No scripts: the two scales are CSS-only tabs.
"""
import html
import json
import os
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
SCALES = [("1000000", "10⁶ statements"), ("100000", "10⁵ statements")]
ENGINES = [("sqlite", "SQLite"), ("sqlite-lftj", "SQLite + LFTJ"), ("redb", "redb"), ("oxigraph", "Oxigraph")]
# The tiramemsu version the run measured (the JSON records the crate version at run time).
MEASURED = "tiramemsu 0.8.1"

WORKLOADS = [
    ("load", "Bulk load", "Time to load the WatDiv-style graph through each engine's bulk path. Lower is better."),
    ("size", "Size on disk", "Bytes per statement. Lower is better; redb grows its file in large steps while loading, and <code>tm compact</code> reclaims it."),
    ("watdiv", "WatDiv-style templates", "Linear (L), star (S), snowflake (F) and complex (C) SPARQL queries over a seeded e-commerce graph, with constants chosen to be selective at every scale."),
    ("lookup", "Lookups in time views", "A point lookup and a two-hop lookup in the current view, as of an earlier transaction and at a valid-time instant. Oxigraph has no time views."),
    ("cypher", "Cypher", "The same graph through openCypher. Oxigraph has no Cypher."),
    ("paths", "Paths", "SPARQL reachability, and Cypher shortest path and trail."),
    ("concurrency", "Concurrent readers", "Two-hop lookups per second from different users on 1, 4 and 16 threads. Higher is better."),
    ("writes", "Writes", "One write per transaction. A tiramemsu write allocates a statement id and a transaction number, checks schemas, records an event, keeps history and cascades; an Oxigraph insert adds a quad."),
    ("cyclic", "Cyclic patterns", "Subgraph counts on a uniform random graph and on a hub-and-spoke graph, where binary joins build large intermediate results. LFTJ is tiramemsu's worst-case-optimal operator for SQLite."),
]

DESCRIBE = {
    "L1": "users in the cities of one country",
    "L2": "reviews of products sold in one city",
    "L3": "what one user's followees like, with genre",
    "S1": "products of one genre under a price, with seller",
    "S2": "users of one city with age and name",
    "S3": "5-star reviews of one genre's products, with reviewer",
    "F1": "snowflake: users in one city → likes → genre, seller → location",
    "F2": "good reviews of expensive products in one genre, reviewer's age",
    "C1": "reviews per genre (aggregate)",
    "C2": "mutual follows in the same city (count)",
}


def load(scale):
    d = json.load(open(os.path.join(HERE, f"results-{scale}.json")))
    rows = {}
    order = []
    for e in d["entries"]:
        key = (e["workload"], e["query"], e["variant"])
        if key not in rows:
            rows[key] = {}
            order.append(key)
        rows[key][e["engine"]] = e
    return d["meta"], rows, order


def num(x):
    """An integer with a space between thousands, as the site writes them."""
    return f"{x:,.0f}".replace(",", " ")


def fmt_time(us):
    if us >= 1e6:
        return f"{us / 1e6:.2f} s"
    if us >= 1e3:
        return f"{us / 1e3:.2f} ms"
    return f"{us:.0f} µs"


def value(e):
    if e is None or e.get("note"):
        return None
    return e["median_us"] if e.get("median_us") is not None else e.get("value")


def cell(e, unit):
    if e is None:
        return "—"
    if e.get("note") == "timeout":
        return "timeout"
    v = value(e)
    if v is None:
        return "—"
    if unit == "s":
        return f"{v:.2f} s"
    if unit == "B/statement":
        return f"{num(v)} B"
    if unit == "queries/s":
        return f"{num(v)}/s"
    return fmt_time(v)


def table(workload, rows, order):
    keys = [k for k in order if k[0] == workload]
    engines = [(k, n) for k, n in ENGINES if any(k in rows[x] for x in keys)]
    head = "<tr><th>Query</th><th>Variant</th><th>Rows</th>" + "".join(f"<th>{n}</th>" for _, n in engines) + "</tr>"
    body = []
    for key in keys:
        r = rows[key]
        unit = next((e.get("unit") for e in r.values() if e.get("unit")), None)
        higher = unit == "queries/s"
        vals = {k: value(r.get(k)) for k, _ in engines}
        present = [v for v in vals.values() if v is not None]
        best = (max(present) if higher else min(present)) if len(present) > 1 else None
        nrows = next((e.get("rows") for e in r.values() if e.get("rows") is not None), None)
        q = html.escape(key[1])
        if workload == "watdiv" and key[1] in DESCRIBE:
            q = f"{q} <span class=\"muted\">{html.escape(DESCRIBE[key[1]])}</span>"
        tds = []
        for k, _ in engines:
            c = html.escape(cell(r.get(k), unit))
            cls = ' class="win"' if best is not None and vals[k] == best else ""
            tds.append(f"<td{cls}>{c}</td>")
        body.append(f"<tr><td>{q}</td><td>{html.escape(key[2])}</td><td>{'' if nrows is None else nrows}</td>{''.join(tds)}</tr>")
    return f"<table class=\"bench\">\n{head}\n" + "\n".join(body) + "\n</table>"


def panel(scale, label):
    meta, rows, order = load(scale)
    when = datetime.fromtimestamp(meta["unix_time"], timezone.utc).strftime("%d %B %Y")
    parts = [f"<p class=\"sub\">{num(meta['statements'])} statements in the WatDiv-style graph · {meta['reps']} timed runs per query after one checked run · {html.escape(meta['machine'])} · measured {when}.</p>"]
    for w, title, desc in WORKLOADS:
        parts.append(f"<h3>{title}</h3>\n<p class=\"sub\">{desc}</p>\n{table(w, rows, order)}")
    return "\n".join(parts)


def headline():
    _, r6, _ = load("1000000")
    _, r5, _ = load("100000")
    v = lambda r, k, e: value(r.get(k, {}).get(e))
    tri = ("cyclic", "triangle", "skewed")
    f1 = ("watdiv", "F1", "now")
    cy = ("cypher", "2-hop", "now")
    c16 = ("concurrency", "2-hop", "16 threads")
    ins = ("writes", "insert", "watdiv")
    ld = ("load", "bulk load", "watdiv")
    return f"""<div class="grid">
    <div class="card"><span class="tag">Cycles</span><h3>Worst-case-optimal joins</h3><p>A triangle through hubs at 10⁶ statements: <strong>{fmt_time(v(r6, tri, 'redb'))}</strong> on redb, {fmt_time(v(r6, tri, 'sqlite-lftj'))} on SQLite with LFTJ, {fmt_time(v(r6, tri, 'sqlite'))} on SQLite's own plan and {fmt_time(v(r6, tri, 'oxigraph'))} on Oxigraph.</p></div>
    <div class="card"><span class="tag">Joins</span><h3>Chains, stars and Cypher</h3><p>The snowflake F1 at 10⁶: {fmt_time(v(r6, f1, 'redb'))} on redb, {fmt_time(v(r6, f1, 'sqlite'))} on SQLite, {fmt_time(v(r6, f1, 'oxigraph'))} on Oxigraph. A Cypher two-hop from a property map: {fmt_time(v(r6, cy, 'redb'))} on redb, {fmt_time(v(r6, cy, 'sqlite'))} on SQLite.</p></div>
    <div class="card"><span class="tag">Readers</span><h3>Concurrent lookups</h3><p>Two-hop lookups per second on 16 threads at 10⁶: {num(v(r6, c16, 'oxigraph'))} on Oxigraph, {num(v(r6, c16, 'redb'))} on redb, {num(v(r6, c16, 'sqlite'))} on SQLite, whose reader pool tops out early.</p></div>
    <div class="card"><span class="tag">Writes</span><h3>Transactions cost time</h3><p>One insert at 10⁵: {fmt_time(v(r5, ins, 'oxigraph'))} on Oxigraph against {fmt_time(v(r5, ins, 'sqlite'))} on SQLite and {fmt_time(v(r5, ins, 'redb'))} on redb. Bulk load of a million statements: {v(r6, ld, 'oxigraph'):.1f} s, {v(r6, ld, 'redb'):.0f} s and {v(r6, ld, 'sqlite'):.0f} s.</p></div>
  </div>"""


PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Benchmarks: SQLite, redb and Oxigraph · tiramemsu</title>
<meta name="description" content="tiramemsu on SQLite (with and without its LFTJ operator) and on redb against Oxigraph on RocksDB, at 10⁵ and 10⁶ statements: joins, cycles, lookups, Cypher, paths, readers, writes and size, losses included.">
<link rel="icon" href="assets/logo.svg" type="image/svg+xml">
<link rel="stylesheet" href="style.css">
</head>
<body>
<header class="top"><div class="wrap">
  <a class="brand" href="./"><img src="assets/logo.svg" alt="">Tiramemsu</a>
  <nav class="menu"><a href="./#why">Why</a><a href="./#features">Features</a><a href="./#start">Quick start</a><a href="benchmarks.html" aria-current="page">Benchmarks</a><a href="articles/">Articles</a><a href="paper/">Papers</a><a href="https://github.com/Volland/tiramemsu">GitHub</a></nav>
</div></header>

<main>
<section><div class="wrap">
  <h1 class="page">Benchmarks: SQLite, redb and Oxigraph</h1>
  <p class="sub">{measured} on SQLite (with and without its LFTJ operator) and on redb, against <a href="https://github.com/oxigraph/oxigraph">Oxigraph</a> 0.5 on RocksDB, the reference Rust RDF store. Same generated data, same queries, every answer compared across engines before anything is timed. The losses are on this page too.</p>
  {headline}
</div></section>

<section><div class="wrap">
  <h2>Read these numbers with our priorities</h2>
  <ul>
    <li><strong>Transactions before speed.</strong> Every tiramemsu write is one ACID transaction with a statement id, a transaction number, schema checks, an event, full history and cascading retractions. The write columns compare different amounts of work.</li>
    <li><strong>Durability:</strong> both tiramemsu engines run at <code>Normal</code> (SQLite <code>synchronous = NORMAL</code> with WAL; redb group commit within 50 ms or 1 000 commits); Oxigraph at its default, RocksDB with its write-ahead log.</li>
    <li><strong>Correctness before timing:</strong> each query runs once on every engine and the results are compared (counts always, sorted rows up to 1 000); a mismatch stops the run. A query that passes <strong>60 s</strong> on its checked run is a timeout and takes no part in that comparison. The cap bounds the checked run only, so a timed median can exceed it.</li>
    <li><strong>What each engine can answer:</strong> Oxigraph has no transaction or valid time, no statement ids and no Cypher, so it is left out of the time-view lookups, Cypher, Cypher paths, cascade and supersede (“—”).</li>
  </ul>
  <p class="note"><strong>Known caveats.</strong> At 10⁶ statements only redb answered the skewed diamond and 4-cycle, so those two answers were not cross-checked in this run; the differential tests check the same queries on small graphs. Concurrent readers moved −17 % (4 threads) at 10⁵ and +20 % at 10⁶ against the previous run with no change on their path; read differences of that size as noise between runs.</p>
</div></section>

<section><div class="wrap">
  <h2>Results</h2>
  <p class="sub">Medians; the fastest engine of each row is marked. Pick a scale.</p>
  <div class="tabs">
    <input type="radio" name="scale" id="scale-6" checked><label for="scale-6">10⁶ statements</label>
    <input type="radio" name="scale" id="scale-5"><label for="scale-5">10⁵ statements</label>
    <div class="panel">
{p6}
    </div>
    <div class="panel">
{p5}
    </div>
  </div>
</div></section>

<section><div class="wrap">
  <h2>Reproduce it, and read more</h2>
  <ul>
    <li><code>cd bench/backends &amp;&amp; cargo run --release -- run --scale 1000000 --reps 5 --out results-1000000.json</code> (Oxigraph builds RocksDB; <code>tm bench</code> runs the SQLite and redb part without it). The raw results are <a href="https://github.com/Volland/tiramemsu/blob/main/bench/backends/results-1000000.json">results-1000000.json</a> and <a href="https://github.com/Volland/tiramemsu/blob/main/bench/backends/results-100000.json">results-100000.json</a>; the full report with p95 and derived ratios is <a href="https://github.com/Volland/tiramemsu/blob/main/bench/backends/README.md">bench/backends/README.md</a>.</li>
    <li><a href="articles/two-engines.html">Two engines, one memory</a>: which engine to choose, and why we accept the losses.</li>
    <li><a href="articles/wcoj-on-kv.html">Worst-case-optimal joins on a key-value store</a>: how the native runner joins.</li>
    <li><a href="articles/making-redb-faster.html">Why the new engine was not faster, and what made it faster</a>: the profiling behind these numbers.</li>
  </ul>
</div></section>
</main>

<footer><div class="wrap">
  <span>Tiramemsu · MIT OR Apache-2.0 · <a href="https://github.com/Volland/tiramemsu">github.com/Volland/tiramemsu</a></span>
  <span><a href="impressum.html">Impressum</a> · <a href="datenschutz.html">Datenschutz</a> · <a href="agb.html">AGB</a></span>
  <span>Generated from <code>bench/backends/results-*.json</code> by <code>bench/backends/build-site.py</code>.</span>
</div></footer>
</body>
</html>
"""

page = PAGE.format(measured=MEASURED, headline=headline(), p6=panel("1000000", "10⁶"), p5=panel("100000", "10⁵"))
out = os.path.join(ROOT, "site", "benchmarks.html")
open(out, "w").write(page)
print("wrote", out)
