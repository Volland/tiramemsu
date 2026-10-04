# Substack long read: Tiramemsu

A long read (about 5,200 words, 10 illustrations) introducing Tiramemsu: layered graphs, the two clocks, never forget, sources and provenance, cited answers, time-respecting paths, fact bundles, the internals and the current status.

| File | What it is |
|---|---|
| `tiramemsu-long-read.md` | The article, the source of truth |
| `tiramemsu-metagraph.md` | Follow-up **draft**: the full metagraph (edges as containers, nesting, n-ary edges, fold/unfold). Describes proposed lifts as well as what works today; its `images/mg-*.png` figures are placeholders not yet drawn |
| `preview.html` | A self-contained preview with the images embedded. Open it in a browser, select everything and paste it into the Substack editor |
| `images/*.png` | The illustrations at 1456 px wide, Substack's recommended width |
| `images/src/*.svg` | Their editable sources, in the site's tiramisu palette |
| `preview.css`, `build-preview.sh` | Rebuild the preview after editing the Markdown (needs `pandoc`) |

To re-render the images after editing an SVG:

```sh
cd article/images && for f in src/*.svg; do rsvg-convert -w 1456 "$f" -o "$(basename "$f" .svg).png"; done
```

## Publishing on Substack

1. **Title:** Tiramemsu: memory that remembers being wrong
2. **Subtitle:** Layered graphs, two clocks and a rule against forgetting. An embedded graph database on SQLite for agents that need to say how sure they are, where they read something, and what they believed last Tuesday.
3. **Cover / social image:** `images/01-cover.png`.
4. **Body:** paste from `preview.html`, then delete the title and subtitle lines at the top, because Substack has its own fields for them. Check that code blocks came through as code blocks. If any image didn't paste, upload it from `images/`.
5. **Captions:** the italic line under each image is its caption. Move it into Substack's image caption field.
6. **Alt text:** each image's alt text is in the Markdown (`![alt](…)`). Paste it into Substack's alt-text field.
7. **Links:** the repository link points to `github.com/Volland/tiramemsu`. Check that it's public before you publish.

Substack has no tables, so the article uses lists and images where the repository docs use tables.

### Other title options

- Layers, two clocks and no delete button: building agent memory on SQLite
- Your agent's brain loves tiramisu
- What did your agent believe last Tuesday?

### SEO description (under 160 characters)

Tiramemsu is an embedded graph database for agent memory: facts with ids, layers of provenance and belief, two clocks, and nothing ever deleted.

### Short social post

> Ask your agent where Alice works. Then ask how sure it is, where it read that, and what it believed last Tuesday. I built Tiramemsu so agents can answer all of those: facts with ids, layers of belief, two clocks, and no delete button. On SQLite.

## Keeping it accurate

Every number and query comes from `lat.md/` and the README as of 2026-09-30: the status table, `lat.md/recipes.md`, `lat.md/time-model.md`, `lat.md/prior-art.md` (DuckDB figures) and the site's quick start. If any of these change before you publish, update the article too. The "Where it stands" section and the known limits go out of date fastest.
