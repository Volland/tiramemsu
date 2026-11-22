#!/usr/bin/env python3
"""Rebuild site/articles/transitional-modeling.html from article.md.

Run from this folder: python3 build-site.py   (needs pandoc)
The article has no images. Its code is crates/tm-transitional and its results
come from crates/tm-transitional/tests/archie.rs.
"""
import os, re, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
OUT = os.path.join(ROOT, "site", "articles", "transitional-modeling.html")
DATE = "22 November 2026"
TITLE = "Who said what, when, and how sure: transitional modeling in an agent's memory"
DESC = ("Posits and assertions after Rönnbäck's transitional modeling, built on tiramemsu statements: "
        "opinions with signed reliability, two time axes, a boundary rule, and information in effect "
        "resolved on read, checked against the paper's Archie example.")

body = subprocess.run(
    ["pandoc", "article.md", "-f", "markdown-implicit_figures", "-t", "html5",
     "--no-highlight", "--wrap=none"],
    cwd=HERE, check=True, capture_output=True, text=True).stdout
body = re.sub(r'^<h1[^>]*>.*?</h1>\s*', '', body, flags=re.S)
body = re.sub(r' id="[^"]*"', '', body)

page = f'''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Transitional modeling: tiramemsu</title>
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

{body}
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
