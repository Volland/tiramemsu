# Article: From datoms to graphs

A long read (about 3,700 words, 5 illustrations), on Substack and on the website as `site/articles/datoms-to-graphs.html`, for developers who know Datomic or EAV. The first half sets up a Datomic-style EAV database in Tiramemsu and runs the usual queries in Datalog: find by attribute, cardinality one, as-of and history, transaction metadata, recursive rules, `with`. The second half shows what statement ids and valid time add: edge properties without a join entity, provenance and graphs per fact, interval-set joins and both clocks, constraints refused at commit, materialized conclusions that fall with their premises, and the same facts from SPARQL and Cypher. It ends with what Datomic has that Tiramemsu does not.

| File | What it is |
|---|---|
| `eav-graphs.md` | The article, the source of truth |
| `preview.html` | Self-contained preview with the images embedded: open it, select all, paste into Substack |
| `build-preview.sh` | Rebuilds `preview.html` (needs `pandoc`; uses `../preview.css`) |
| `build-site.py` | Rebuilds the website page `site/articles/datoms-to-graphs.html` and copies the SVGs to `site/assets/eav-graphs/` (needs `pandoc`): `python3 build-site.py` |
| `images/*.png` | The illustrations at 1456 px wide |
| `images/src/*.svg` | Their editable sources, in the tiramisu palette |

Re-render the images after editing an SVG, then run `python3 build-site.py` so the site gets the new SVGs:

```sh
cd images && for f in src/*.svg; do rsvg-convert -w 1456 "$f" -o "$(basename "$f" .svg).png"; done
```

## Where the claims come from

Every Tiramemsu query and result in the article is a test in `crates/tiramemsu/tests/eav_graphs.rs`, one per section, run on SQLite and (with `redb`) on redb:

- "Find by attribute": `find_by_attribute`
- "Cardinality one, as-of and history": `cardinality_one_and_as_of`
- "Who wrote it": `transaction_metadata` (pins `stmt:30`, `tx:3`)
- "Rules": `recursive_rules_over_refs`
- "What if": `what_if_with`
- "Power-up 1: a fact you can point at": `edge_properties` (Datalog, SPARQL annotation, Cypher; pins `stmt:35`, `stmt:38`)
- "Power-up 2: provenance per fact": `graphs_and_confidence` (pins `stmt:43`, `tx:5`)
- "Power-up 3: the second clock": `valid_time_and_two_clocks`
- "Power-up 4: rules that refuse a commit": `one_lead_at_a_time`
- "Power-up 5: conclusions that remember why": `materialized_with_support`
- "One store, every surface": `same_facts_from_sparql` (property path, `SERVICE`, Cypher `CALL`)

Run them with `cargo test -p tiramemsu --features datalog --test eav_graphs`. The statement and transaction ids in the text depend on the order of writes in the fixture: adding a statement to it shifts them, and the test pins them so the drift is caught. The Rust snippets are trimmed from the fixture (`sys_author` stands for `Value::iri(vocab::SYS_AUTHOR)`, `ms` parses a date). The Datomic snippets are illustrative and are not run.

The benchmark figures (84× the N3 path on a hub closure, 1.0–1.4× Soufflé's interpreter) come from `lat.md/datalog.md#Gates And Oracles` and `bench/datalog/README.md`, run of 2026-10-29. The limits section follows the design notes: decision DL3 (text syntax), DL22 (incremental runs, not built), `time-model#Erasure` (crypto-shredding, milestone M6) and `data-model#Named Graphs` (the 1.9× cost figure).

## Publishing on Substack

1. **Title:** From datoms to graphs
2. **Subtitle:** Datomic-style EAV with Datalog, and what a statement id adds.
3. **Cover / social image:** `images/01-datom-vs-statement.png`.
4. **Body:** run `./build-preview.sh`, paste from `preview.html`, then delete the title and the italic standfirst if you put them in Substack's own fields. Check that code blocks came through as code blocks; upload any image that did not paste.
5. **Captions and alt text:** the italic line under each image is its caption, and each image's alt text is in the Markdown.

Substack has no tables, so the article uses lists.

### Other title options

- EAV was always a graph
- Datomic's model, plus an id per fact and a second clock
- What a statement id buys you: from datoms to a temporal graph

### SEO description (under 160 characters)

Datomic-style EAV and Datalog in Tiramemsu, then what statement ids and valid time add: edge properties, per-fact provenance, temporal joins, constraints.

### LinkedIn post

> If you have used Datomic, you already think in facts: entity, attribute, value, transaction. No tables, immutable history, Datalog.
>
> I wrote up how the same setup looks in Tiramemsu, the embedded graph database I'm building, query by query: find by attribute, cardinality one, as-of, history, who wrote what, recursive rules, speculative writes.
>
> Then the part Datomic can't do, because a datom has no identity and only one clock:
> 🔹 an edge with properties, without turning it into an entity
> 🔹 a source and a confidence on one fact, not on a whole transaction
> 🔹 named graphs as tags, so a rule can trust only HR's facts
> 🔹 valid time in every join: "who worked with Ada, and when?" with no date filter
> 🔹 "what was true on 1 April" and "what did we know on 1 April" as two different answers
> 🔹 a one-lead-at-a-time constraint that refuses the commit
> 🔹 derived facts that cite their premises and fall when a premise is retracted
>
> It also lists what Datomic has that this doesn't. Every query in it is a test in the repo.
>
> 🔗 [link]
>
> #Datalog #Datomic #KnowledgeGraphs #Bitemporal #Rust #AIAgents

### Short social post

> EAV was always a graph. Give every fact an id and a valid-time interval, and you get edge properties, per-fact provenance and temporal joins for free. Datomic-style Datalog on Tiramemsu, and what changes.
