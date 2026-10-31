# Article and guide: ELPI in Tiramemsu

A long read (about 2,000 words, no images, no tables) on adding ELPI (λProlog) as Tiramemsu's third reasoning language: host callbacks instead of copies, typed terms, support the program states, valid time by intersection, delta-seeded constraints, three things the tests found, and one call path for goal calls. It appears on Substack, from this folder, and on the website as `site/articles/elpi-reasoning.html`.

The guide `docs/elpi.md` (calls, terms, time, constraints, stored programs, materialization, goal calls, errors, limits) is rendered to `site/elpi/index.html`.

| File | What it is |
|---|---|
| `elpi-reasoning.md` | The article, the source of truth |
| `build-site.py` | Rebuilds the article page (needs `pandoc`): `python3 build-site.py` |
| `build-guide.py` | Rebuilds the guide page from `docs/elpi.md`: `python3 build-guide.py` |

## Where the claims come from

- Decisions: `lat.md/elpi.md` (E1 to E13).
- Tests named in the text: `crates/tiramemsu/tests/elpi.rs` (scenarios, `datalog_oracle_agrees`), `elpi_props.rs` (`delta_refuses_exactly_new_violations`), `elpi_calls.rs`.
- lean-elpi's speed relative to ELPI 1.18: lean-elpi's own benchmark. No tiramemsu ELPI benchmark exists yet; when one does, quote it here.
