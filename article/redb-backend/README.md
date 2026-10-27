# Articles: the redb backend

Two articles on the second storage engine (`add-redb-backend`, `add-native-runner`, `add-backend-benchmarks`). They appear on Substack, from this folder, and on the website as `site/articles/two-engines.html` and `site/articles/wcoj-on-kv.html`.

| File | What it is |
|---|---|
| `two-engines.md` | "Two engines, one memory": why a second engine, one logical database in both, durability, which engine to choose, and the benchmark with its losses |
| `wcoj-on-kv.md` | "Worst-case-optimal joins on a key-value store": how other engines join, the native runner, and the four performance bugs the benchmark found |
| `build-site.py` | Rebuilds both website pages (needs `pandoc`) |

Every number comes from `bench/backends/results-*.json` and is reproduced with `cd bench/backends && cargo run --release -- run --scale 100000 --reps 5`. No tables in the Markdown, because Substack cannot show them; the tables are in `bench/backends/README.md`.
