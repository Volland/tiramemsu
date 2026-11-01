#!/usr/bin/env python3
"""Rebuild site/elpi/index.html from docs/elpi.md.

Run from this folder: python3 build-guide.py   (needs pandoc)
"""
import os, re, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
SRC = os.path.join(ROOT, "docs", "elpi.md")
OUT = os.path.join(ROOT, "site", "elpi", "index.html")
TITLE = "ELPI in Tiramemsu: a guide"
DESC = ("A practical guide to ELPI (λProlog) in Tiramemsu: host builtins over a view, terms, valid time, "
        "constraints enforced at commit, stored programs with includes, materialization, and goal calls from SPARQL and Cypher.")

body = subprocess.run(["pandoc", SRC, "-f", "markdown-implicit_figures", "-t", "html5", "--no-highlight", "--wrap=none"],
                      check=True, capture_output=True, text=True).stdout
body = re.sub(r'^<h1[^>]*>.*?</h1>\s*', '', body, flags=re.S)
body = body.replace("../site/articles/elpi-reasoning.html", "../articles/elpi-reasoning.html")

page = f'''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{TITLE}</title>
<meta name="description" content="{DESC}">
<link rel="icon" href="../assets/logo.svg" type="image/svg+xml">
<link rel="stylesheet" href="../style.css">
</head>
<body>
<header class="top"><div class="wrap">
  <a class="brand" href="../"><img src="../assets/logo.svg" alt="">Tiramemsu</a>
  <nav class="menu"><a href="../#features">Features</a><a href="../#start">Quick start</a><a href="../datalog/">Datalog guide</a><a href="./" aria-current="page">ELPI guide</a><a href="../n3/">N3 book</a><a href="../articles/">Articles</a></nav>
</div></header>

<article class="post">
<h1>{TITLE}</h1>
<p class="meta">Guide · feature <code>elpi</code> · crates <code>tm-elpi</code>, <code>tm-elpi-store</code></p>

{body}
<p><a href="../articles/elpi-reasoning.html">The long read: λProlog in a bitemporal memory →</a></p>
</article>

<footer><div class="wrap">
  <span>Tiramemsu · MIT OR Apache-2.0 · <a href="https://github.com/Volland/tiramemsu">github.com/Volland/tiramemsu</a></span>
  <span><a href="../">Home</a> · <a href="../articles/">Articles</a></span>
  <span><a href="../impressum.html">Impressum</a> · <a href="../datenschutz.html">Datenschutz</a> · <a href="../agb.html">AGB</a></span>
</div></footer>
</body>
</html>
'''
os.makedirs(os.path.dirname(OUT), exist_ok=True)
open(OUT, "w").write(page)
print("wrote", OUT)
