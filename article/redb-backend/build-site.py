#!/usr/bin/env python3
"""Rebuild the redb-backend article pages in site/articles/ from the Markdown.

Run from this folder: python3 build-site.py   (needs pandoc)
"""
import os, re, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
DATE = "27 October 2026"

PAGES = [
    ("two-engines.md", "two-engines.html", "Two engines, one memory",
     "Tiramemsu stores a memory in SQLite or in redb: one logical database in both, why we added a second engine, "
     "and what the measurements against SQLite and Oxigraph say about each, losses included.",
     ("wcoj-on-kv.html", "Next: worst-case-optimal joins on a key-value store →")),
    ("wcoj-on-kv.md", "wcoj-on-kv.html", "Worst-case-optimal joins on a key-value store",
     "How tiramemsu's native runner joins SPARQL and Cypher patterns over redb, what it borrows from RDF-3X, Kùzu "
     "and Umbra, and the performance bugs the benchmark found in it.",
     ("making-redb-faster.html", "Next: why the new engine was not faster, and what made it faster →")),
    ("making-redb-faster.md", "making-redb-faster.html", "Why the new engine was not faster, and what made it faster",
     "Profiling tiramemsu's redb engine against SQLite: values decoded a byte at a time, allocations per row, "
     "a join order that started at the wrong end and filters that never reached their pattern.",
     ("two-engines.html", "Related: two engines, one memory →")),
]

TEMPLATE = '''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}: tiramemsu</title>
<meta name="description" content="{desc}">
<link rel="icon" href="../assets/logo.svg" type="image/svg+xml">
<link rel="stylesheet" href="../style.css">
</head>
<body>
<header class="top"><div class="wrap">
  <a class="brand" href="../"><img src="../assets/logo.svg" alt="">Tiramemsu</a>
  <nav class="menu"><a href="../#features">Features</a><a href="../#start">Quick start</a><a href="./" aria-current="page">Articles</a><a href="../paper/">Papers</a><a href="https://github.com/Volland/tiramemsu">GitHub</a></nav>
</div></header>

<article class="post">
<h1>{title}</h1>
<p class="meta">Tiramemsu articles · {date}</p>

{body}
<p><a href="{next_href}">{next_text}</a></p>
</article>

<footer><div class="wrap">
  <span>Tiramemsu · MIT OR Apache-2.0 · <a href="https://github.com/Volland/tiramemsu">github.com/Volland/tiramemsu</a></span>
  <span><a href="../">Home</a> · <a href="./">All articles</a></span>
  <span><a href="../impressum.html">Impressum</a> · <a href="../datenschutz.html">Datenschutz</a> · <a href="../agb.html">AGB</a></span>
</div></footer>
</body>
</html>
'''

for src, out, title, desc, (next_href, next_text) in PAGES:
    body = subprocess.run(
        ["pandoc", src, "-f", "markdown-implicit_figures", "-t", "html5", "--no-highlight", "--wrap=none"],
        cwd=HERE, check=True, capture_output=True, text=True).stdout
    # the title lives in the page header; the italic standfirst stays
    body = re.sub(r'^<h1[^>]*>.*?</h1>\s*', '', body, flags=re.S)
    body = re.sub(r'<hr />\s*', '', body, count=1)
    body = re.sub(r' id="[^"]*"', '', body)
    page = TEMPLATE.format(title=title, desc=desc, date=DATE, body=body,
                           next_href=next_href, next_text=next_text)
    path = os.path.join(ROOT, "site", "articles", out)
    open(path, "w").write(page)
    print("wrote", path)
