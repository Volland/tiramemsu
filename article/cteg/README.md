# Article: causal-temporal event graphs on Tiramemsu

An article (about 4,500 words, 8 illustrations) on causal-temporal event graphs (CTEGs), the model from Simon Foldvik's paper [arXiv 2604.17557](https://arxiv.org/abs/2604.17557), and how to build, check and query them in SPARQL on Tiramemsu. The working example is the real execution trace of the Claude Code session that wrote the article, including a fact-checking subagent.

It appears in two places: on Substack, from this folder, and on the website as `site/articles/causal-temporal-event-graphs.html`.

| File | What it is |
|---|---|
| `cteg-on-tiramemsu.md` | The article, the source of truth for Substack. No tables, because Substack cannot show them |
| `preview.html` | A self-contained preview with the images embedded. Open it, select everything and paste into the Substack editor |
| `images/*.png` | The illustrations at 1456 px wide |
| `images/src/*.svg` | Their sources, in the site's tiramisu palette. `01`, `05`, `06` and `08` are generated; the rest are drawn by hand |
| `code/extract-trace.py` | Turns a Claude Code transcript (and subagent transcripts) into a sanitized trace: ids, parents, times, types, tool names and short labels only |
| `code/trace.json` | The sanitized trace used in the article: 205 main-agent records and 65 subagent records |
| `code/cteg-demo.mjs` | Loads the trace into Tiramemsu as a CTEG through the Node binding and runs every query in the article. `RECORDER=sparql` records it through SPARQL `INSERT DATA` with valid-time annotations instead; queries 1 to 12 give the same answers |
| `code/queries.out.txt` | The output of the last run |
| `code/gen-figures.py` | Draws the data figures from `trace.json` |
| `build-preview.sh`, `../preview.css` | Rebuild the preview (needs `pandoc`) |
| `build-site.py` | Rebuild the website page and copy the SVGs to `site/assets/cteg/` (needs `pandoc`) |

## Rebuilding

```sh
cd article/cteg/code
node cteg-demo.mjs trace.json /tmp/cteg.db > queries.out.txt      # needs the Node binding built in bindings/node (0.6 + valid-time writes)
RECORDER=sparql node cteg-demo.mjs trace.json /tmp/cteg-sparql.db  # the same trace written through SPARQL
python3 gen-figures.py trace.json ../images/src
cd ../images && for f in src/*.svg; do rsvg-convert -w 1456 "$f" -o "$(basename "$f" .svg).png"; done
cd .. && ./build-preview.sh && python3 build-site.py
```

Transaction numbers and `tm:addedAt` instants depend on when the loader runs; everything else (valid times, counts, causal structure) comes from the trace and is stable. If you re-run, update the graft transaction's `addedAt` quoted in the article.

## Publishing on Substack

1. **Title:** Causal-temporal event graphs on Tiramemsu
2. **Subtitle:** An agent's trace should say what caused what, not just what came next. Building CTEGs on statements with ids and two clocks, checked and queried in SPARQL, on the real trace of the agent that wrote this article.
3. **Cover / social image:** `images/01-cover.png`.
4. **Body:** paste from `preview.html`, then delete the title and subtitle at the top. Check that code blocks came through as code. Upload any image that did not paste from `images/`.
5. **Captions:** the italic line under each image is its caption; move it into Substack's caption field. Alt text is in the Markdown (`![alt](…)`).

### Other title options

- Order is not cause: agent traces as causal-temporal event graphs
- What caused what: putting agent traces into a bitemporal graph
- The trace of the agent that wrote this article

### SEO description

Causal-temporal event graphs for agent traces, built on statement ids, valid time and transaction time in Tiramemsu, checked and queried in SPARQL.

### Short social post

> A log tells you what came next. A causal-temporal event graph tells you what caused what. I loaded the real trace of the agent that wrote this article into Tiramemsu: 77 links in the raw log go backwards in time, two parenthood rules fix all of them, and SPARQL answers "why did this happen" across a subagent boundary.

## Keeping it accurate

Claims about the paper were checked against arXiv 2604.17557v1 (19 April 2026) by a separate fact-checking pass. Query syntax follows `lat.md/query.md` (Temporal Syntax, Temporal Path Syntax, SPARQL 1.2 reifiers). Every SPARQL block in the Markdown was run verbatim against the loaded trace.
