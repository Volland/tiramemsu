#!/usr/bin/env python3
"""Rebuild site/articles/elpi-validators.html from the Markdown.

Run from this folder: python3 build-site.py   (needs pandoc)
The article has no images.
"""
import os, re, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
OUT = os.path.join(ROOT, "site", "articles", "elpi-validators.html")
DATE = "31 October 2026"
TITLE = 'Validators that ask “what if”'
DESC = "A hands-on tour of ELPI in Tiramemsu: questions over memory, hypothetical answers with =>, policies as predicates, pi for any member, validators that refuse a bad write, and grounded conclusions. Preview: not yet released."

body = subprocess.run(
    ["pandoc", "elpi-validators.md", "-f", "markdown-implicit_figures", "-t", "html5",
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
<title>Validators that ask “what if”: tiramemsu</title>
<meta name="description" content="{DESC}">
<link rel="icon" href="../assets/logo.svg" type="image/svg+xml">
<link rel="stylesheet" href="../style.css">
</head>
<body>
<header class="top"><div class="wrap">
  <a class="brand" href="../"><img src="../assets/logo.svg" alt="">Tiramemsu</a>
  <nav class="menu"><a href="../#features">Features</a><a href="../#start">Quick start</a><a href="./" aria-current="page">Articles</a><a href="../datalog/">Datalog</a><a href="../elpi/">ELPI</a><a href="../paper/">Papers</a><a href="https://github.com/Volland/tiramemsu">GitHub</a></nav>
</div></header>

<article class="post">
<h1>{TITLE}</h1>
<p class="meta">Tiramemsu articles · {DATE}</p>
<p class="meta"><strong>Preview:</strong> ELPI support is built and tested but not yet released; it needs lean-elpi 0.4 on crates.io and will ship in a later release (feature <code>elpi</code>, off by default).</p>

{body}
<p><a href="../elpi/">The ELPI guide →</a> · <a href="elpi-reasoning.html">Related: λProlog in a bitemporal memory →</a></p>
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
