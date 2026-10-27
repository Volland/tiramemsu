# Articles: the redb backend

Three articles on the second storage engine (`add-redb-backend`, `add-native-runner`, `add-backend-benchmarks`, `tune-native-runner`). They appear on Substack, from this folder, and on the website as `site/articles/two-engines.html`, `site/articles/wcoj-on-kv.html` and `site/articles/making-redb-faster.html`.

| File | What it is |
|---|---|
| `two-engines.md` | "Two engines, one memory": why a second engine, one logical database in both, durability, which engine to choose, and the benchmark with its losses |
| `wcoj-on-kv.md` | "Worst-case-optimal joins on a key-value store": how other engines join, the native runner, and the performance bugs the benchmark found |
| `making-redb-faster.md` | "Why the new engine was not faster, and what made it faster": the profiles, value decoding, allocations, the join order, constant equalities, and the rerun benchmark |
| `build-site.py` | Rebuilds the three website pages (needs `pandoc`) |

Every number comes from `bench/backends/results-*.json` and is reproduced with `cd bench/backends && cargo run --release -- run --scale 100000 --reps 5` (and `--scale 1000000`). The numbers are from the run after `tune-native-runner`; `making-redb-faster.md` also quotes the 0.8.0 run before it. No tables in the Markdown, because Substack cannot show them; the tables are in `bench/backends/README.md`.
