#!/bin/sh
# Rebuild preview.html (self-contained, images embedded) from the Markdown.
# Needs pandoc and python3. Run from this folder: ./build-preview.sh
set -e
cd "$(dirname "$0")"
pandoc tiramemsu-long-read.md -f markdown-implicit_figures -t html5 --standalone \
  --embed-resources --metadata pagetitle="Tiramemsu long read" --css preview.css \
  --highlight-style=zenburn -o preview.html
# a line in italics right after an image is its caption
python3 - <<'PY'
import re
s = open("preview.html").read()
s = re.sub(r'(<img[^>]*/?>\s*</p>\s*)<p><em>', r'\1<p class="caption"><em>', s)
open("preview.html", "w").write(s)
PY
