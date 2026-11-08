#!/usr/bin/env python3
"""Rebuild site/articles/datoms-to-graphs.html from the Markdown.

Run from this folder: python3 build-site.py   (needs pandoc)
The figures are the SVGs in images/src, copied to site/assets/eav-graphs/.
"""
import os, re, shutil, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
ASSETS = os.path.join(ROOT, "site", "assets", "eav-graphs")
OUT = os.path.join(ROOT, "site", "articles", "datoms-to-graphs.html")
DATE = "8 November 2026"
TITLE = "From datoms to graphs"
DESC = ("Datomic-style EAV with Datalog in Tiramemsu, query by query, then what statement ids and valid time add: "
        "edge properties, provenance per fact, temporal joins, constraints at commit and conclusions that fall "
        "with their premises.")

os.makedirs(ASSETS, exist_ok=True)
for f in os.listdir(os.path.join(HERE, "images", "src")):
    shutil.copy(os.path.join(HERE, "images", "src", f), ASSETS)

body = subprocess.run(
    ["pandoc", "eav-graphs.md", "-f", "markdown-implicit_figures", "-t", "html5",
     "--no-highlight", "--wrap=none"],
    cwd=HERE, check=True, capture_output=True, text=True).stdout
# the title lives in the page header; the italic standfirst stays
body = re.sub(r'^<h1[^>]*>.*?</h1>\s*', '', body, flags=re.S)
body = re.sub(r'<hr />\s*', '', body, count=1)
sizes = {f: re.search(r'viewBox="0 0 (\d+) (\d+)"', open(os.path.join(ASSETS, f)).read()).groups()
         for f in os.listdir(ASSETS)}

def figure(m):
    src, alt, cap = m.groups()
    name = os.path.basename(src).replace(".png", ".svg")
    w, h = sizes[name]
    return (f'<figure>\n  <div class="diagram"><a href="../assets/eav-graphs/{name}"><img src="../assets/eav-graphs/{name}" '
            f'alt="{alt}" width="{w}" height="{h}"></a></div>\n  <figcaption>{cap}</figcaption>\n</figure>')

body, n = re.subn(r'<p><img src="(images/[^"]+)" alt="([^"]*)" /></p>\s*<p><em>(.*?)</em></p>', figure, body, flags=re.S)
assert n == 5, f"expected 5 figures, found {n}"
body = re.sub(r' id="[^"]*"', '', body)
body = body.replace("https://volland.github.io/tiramemsu/articles/", "")
body = body.replace("https://volland.github.io/tiramemsu/datalog/", "../datalog/")

page = f'''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{TITLE}: tiramemsu</title>
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
<p><a href="layered-graphs.html">Related: layered graphs, explained →</a><br>
<a href="time-travel.html">Related: time travel and bitemporality, explained →</a></p>
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
