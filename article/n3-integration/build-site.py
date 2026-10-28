#!/usr/bin/env python3
"""Rebuild site/articles/n3-reasoner-integration.html from the Markdown.

Run from this folder: python3 build-site.py   (needs pandoc)
The article has no images.
"""
import os, re, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
OUT = os.path.join(ROOT, "site", "articles", "n3-reasoner-integration.html")
DATE = "28 October 2026"
TITLE = 'Putting an N3 reasoner inside a bitemporal memory'
DESC = 'How Tiramemsu integrates the eyeron N3 reasoner: statements as IRIs, pointwise valid time by segment sweep, a pinned clock, rule sets as versioned statements, materialization as grounded derived beliefs, the conformance tests, and the dependency problem that blocks publishing.'

body = subprocess.run(
    ["pandoc", "n3-reasoner-integration.md", "-f", "markdown-implicit_figures", "-t", "html5",
     "--no-highlight", "--wrap=none"],
    cwd=HERE, check=True, capture_output=True, text=True).stdout
# the title lives in the page header; the italic standfirst stays
body = re.sub(r'^<h1[^>]*>.*?</h1>\s*', '', body, flags=re.S)
body = re.sub(r'<hr />\s*', '', body, count=1)
body = re.sub(r' id="[^"]*"', '', body)
body = body.replace("https://volland.github.io/tiramemsu/n3/", "../n3/")

page = f'''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Putting an N3 reasoner inside a bitemporal memory: tiramemsu</title>
<meta name="description" content="{DESC}">
<link rel="icon" href="../assets/logo.svg" type="image/svg+xml">
<link rel="stylesheet" href="../style.css">
</head>
<body>
<header class="top"><div class="wrap">
  <a class="brand" href="../"><img src="../assets/logo.svg" alt="">Tiramemsu</a>
  <nav class="menu"><a href="../#features">Features</a><a href="../#start">Quick start</a><a href="./" aria-current="page">Articles</a><a href="../datalog/">Datalog</a><a href="../paper/">Papers</a><a href="https://github.com/Volland/tiramemsu">GitHub</a></nav>
</div></header>

<article class="post">
<h1>{TITLE}</h1>
<p class="meta">Tiramemsu articles · {DATE}</p>

{body}
<p><a href="../n3/">Related: reasoning with N3 in Tiramemsu (the mini book) →</a> · <a href="datalog-temporal-reasoning.html">Datalog over time →</a></p>
</article>

<footer><div class="wrap">
  <span>Tiramemsu · MIT OR Apache-2.0 · <a href="https://github.com/Volland/tiramemsu">github.com/Volland/tiramemsu</a></span>
  <span><a href="../">Home</a> · <a href="./">All articles</a></span>
  <span><a href="../impressum.html">Impressum</a> · <a href="../datenschutz.html">Datenschutz</a> · <a href="../agb.html">AGB</a></span>
</div></footer>
</body>
</html>
'''
open(OUT, "w").write(page)
print("wrote", OUT)
