#!/bin/sh
# Rebuild preview.html (self-contained, images embedded) from the Markdown,
# for pasting into the Substack editor. Needs pandoc and python3.
# Run from anywhere: ./build-preview.sh
set -e
cd "$(dirname "$0")"
pandoc eav-graphs.md -f markdown-implicit_figures -t html5 --standalone \
  --embed-resources --metadata pagetitle="From datoms to graphs" \
  --css ../preview.css --highlight-style=zenburn -o preview.html
# a line in italics right after an image is its caption
python3 - <<'PY'
import re
s = open("preview.html").read()
s = re.sub(r'(<img[^>]*/?>\s*</p>\s*)<p><em>', r'\1<p class="caption"><em>', s)
open("preview.html", "w").write(s)
PY
