# Article: Putting an N3 reasoner inside a bitemporal memory

A long read (about 2,200 words, no images, no tables) on how Tiramemsu integrates the eyeron N3 reasoner: the pure `tm-n3` crate and the loading facade, statements as IRIs with `tm:content`, pruning, valid time by segment sweep and why intersecting proofs would be unsound, the pinned clock, rule sets as versioned statements, materialization as grounded derived beliefs, what it refuses and why, the conformance tests against eyeron's own examples, and the git-dependency problem that keeps the facade off crates.io.

It appears on Substack, from this folder, and on the website as `site/articles/n3-reasoner-integration.html`.

| File | What it is |
|---|---|
| `n3-reasoner-integration.md` | The article, the source of truth. No tables, because Substack cannot show them |
| `build-site.py` | Rebuilds the website page (needs `pandoc`): `python3 build-site.py` |

## Where the claims come from

- Decisions: `lat.md/reasoning.md` (N1 to N18). Requirements: `openspec/specs/n3-*`.
- Examples: each is named after a test in `crates/tiramemsu/tests/n3_reasoning.rs`, `n3_properties.rs` or `n3_grounding.rs`; the conformance counts come from `crates/tiramemsu/tests/n3_conformance/README.md`.
- The clock: `time:localTime` is still rewritten to `tm:run tm:now` in `crates/tm-n3/src/rules.rs`; the article says so. If the rewrite is replaced by eyeron's `now` option, update the article.
- The dependency story: the comment above `eyeron` in the workspace `Cargo.toml` and `openspec/changes/publish-eyeron-dependency/`.
