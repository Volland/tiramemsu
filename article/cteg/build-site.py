#!/usr/bin/env python3
"""Rebuild site/articles/causal-temporal-event-graphs.html from the Markdown.

Run from this folder: python3 build-site.py   (needs pandoc)
The figures are the SVGs in images/src, copied to site/assets/cteg/.
"""
import os, re, shutil, subprocess

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
ASSETS = os.path.join(ROOT, "site", "assets", "cteg")
OUT = os.path.join(ROOT, "site", "articles", "causal-temporal-event-graphs.html")
DATE = "13 October 2026"

os.makedirs(ASSETS, exist_ok=True)
for f in os.listdir(os.path.join(HERE, "images", "src")):
    shutil.copy(os.path.join(HERE, "images", "src", f), ASSETS)

body = subprocess.run(
    ["pandoc", "cteg-on-tiramemsu.md", "-f", "markdown-implicit_figures", "-t", "html5",
     "--no-highlight", "--wrap=none"],
    cwd=HERE, check=True, capture_output=True, text=True).stdout
# the title, subtitle and cover live in the page header
body = re.sub(r'^<h1[^>]*>.*?</h1>\s*<p><em>.*?</em></p>\s*', '', body, flags=re.S)
body = re.sub(r'<p><img src="images/01-cover.png"[^>]*/></p>\s*<hr />\s*', '', body, flags=re.S)
sizes = {f: re.search(r'viewBox="0 0 (\d+) (\d+)"', open(os.path.join(ASSETS, f)).read()).groups()
         for f in os.listdir(ASSETS)}

def figure(m):
    src, alt, cap = m.groups()
    name = os.path.basename(src).replace(".png", ".svg")
    w, h = sizes[name]
    return (f'<figure>\n  <div class="diagram"><a href="../assets/cteg/{name}"><img src="../assets/cteg/{name}" '
            f'alt="{alt}" width="{w}" height="{h}"></a></div>\n  <figcaption>{cap}</figcaption>\n</figure>')

body = re.sub(r'<p><img src="(images/[^"]+)" alt="([^"]*)" /></p>\s*<p><em>(.*?)</em></p>', figure, body, flags=re.S)
body = re.sub(r' id="[^"]*"', '', body)
body = body.replace("https://volland.github.io/tiramemsu/articles/", "")

desc = ("Causal-temporal event graphs (CTEGs) for agent traces, built on statement ids, valid time and "
        "transaction time in Tiramemsu, written and queried in SPARQL and Cypher on the real trace of the "
        "agent that wrote this article.")
page = f'''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Causal-temporal event graphs: tiramemsu</title>
<meta name="description" content="{desc}">
<link rel="icon" href="../assets/logo.svg" type="image/svg+xml">
<link rel="stylesheet" href="../style.css">
</head>
<body>
<header class="top"><div class="wrap">
  <a class="brand" href="../"><img src="../assets/logo.svg" alt="">Tiramemsu</a>
  <nav class="menu"><a href="../#features">Features</a><a href="../#start">Quick start</a><a href="./" aria-current="page">Articles</a><a href="../datalog/">Datalog</a><a href="../elpi/">ELPI</a><a href="../paper/">Papers</a><a href="https://github.com/Volland/tiramemsu">GitHub</a></nav>
</div></header>

<article class="post">
<h1>Causal-temporal event graphs on Tiramemsu</h1>
<p class="meta">Tiramemsu articles · {DATE}</p>

<p><em>An agent’s trace should say what caused what, not just what came next. Here is how to build causal-temporal event graphs (CTEGs) on statements with ids and two clocks, checked and queried in SPARQL, using the real trace of the agent that wrote this article.</em></p>
<figure>
  <div class="diagram"><img src="../assets/cteg/01-cover.svg" alt="The real execution trace of the session that wrote this article, drawn as a tree on a time axis, with a subagent tree grafted onto one branch." width="1456" height="816"></div>
</figure>

{body}
<p><a href="journeys-in-time.html">Related: journeys that respect time →</a></p>
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
