# Storage engine research: an append-only log with indexes

Research of 2026-10-01. The question: which file format or engine gives tiramemsu a fast, compact, append-only log with indexes, for the data model in `lat.md/data-model.md` and `lat.md/storage.md`.

This is option analysis, not a decision. Claims marked *(unverified)* come from secondary sources; byte estimates marked *(estimate)* are not measured.

## 1. What the data model actually needs

The workload has a narrower shape than general KV storage, and every option below is judged against it.

- **Facts are immutable.** A statement row is inserted once. The only mutation is setting `t_ret`/`ret_kind` once, from NULL.
- **Ids are monotone.** One writer allocates `eid` and `t`, both strictly increasing. So **`t_add` is monotone in `eid`**: "added by `t`" is `eid < E(t)`, where `E(t)` is the first eid of the next transaction.
- **Retractions are also an ordered log.** `t_ret` values arrive in increasing order: the "increasing ending time" interval case.
- **Nine sort orders.** live and history `spo`/`pos`/`osp`, `valid_p`, `log_add` and `log_ret`, served through the `Family` + prefix + bound + `View` contract in `Store/Types.lean`.
- **Contract requirements.** Interactive transactions with read-your-writes, savepoints (`with`, `dry_run`), snapshot readers, and ordered range scans with early exit (`Store/Interface.lean`).
- **Hosts.** rusqlite now; WASM and Durable Objects at the `tm-core` tier (`lat.md/architecture.md#Executor`).
- **Today's numbers.** 153 B/statement, indexes 5.3× the table, 9 B-tree entries per assert. Point lookup 4.2 µs, assert transaction 206 µs, 1.1 M statements in 168.6 MB.

## 2. Engines (Rust-embeddable), status as of 2026-10

| Engine | Kind | Savepoints / nested rollback | Multi-keyspace atomic | WASM | Status | Fit |
|---|---|---|---|---|---|---|
| **SQLite** (today) | B-tree + WAL | yes | yes (tables) | yes | reference | baseline; partial indexes, triggers, SQL codegen, FTS5 |
| **RocksDB** (`rust-rocksdb` 0.25, 2026-08) | leveled LSM | yes, stackable `set_savepoint` | column families + WriteBatch | no (C++) | mature, ~1 release/yr | every feature; slowest small point reads in redb's table |
| **fjall** 3.1.x (2026-08) | LSM, keyspaces share a journal | **no**, so you need your own undo overlay | yes | not documented | active, pure Rust | best pure-Rust LSM; v3 adds prefix truncation and partitioned filters |
| **heed / LMDB** (0.22, 2026-04) | CoW B+tree, mmap | yes, nested write txns | yes (DBIs) | no | mature (Meilisearch) | fastest reads; never shrinks; old versions not kept |
| **libmdbx** (`libmdbx-rs` 0.9, 2026-09) | CoW B+tree | yes | yes | no | used by Reth/Erigon; unusual upstream hosting | like LMDB, with auto-compaction |
| **redb** 4.3 (2026-09) | CoW B+tree | restricted (not after the txn is dirty) | yes | no | stable, active | high write/space amplification (4 GiB uncompacted vs RocksDB 0.89) |
| **SurrealKV** 1.0 rewrite (merged 2026-09-25) | LSM + OCC | not documented | yes | **OPFS in browser** | very new; 1.0 crate not confirmed published | only new engine with a real WASM story |
| **SlateDB** 0.17 | LSM on object storage | — | — | — | near 1.0 | cloud tier, latency in tens of ms; not a local engine |
| **Turso / Limbo** 0.8 | SQLite rewrite, MVCC beta | not confirmed | — | yes | beta | watch; MVCC-mode limits *(unverified)* |
| sled, nebari, agatedb, Speedb, canopydb | — | — | — | — | stale, dead or "do not trust" | rejected |

How other graph stores use these:
- **Oxigraph:** RocksDB with one column family per quad permutation. That is the closest analogue to the nine families. On WASM it falls back to in-memory.
- **SurrealDB 3:** RocksDB in production, SurrealKV, and IndexedDB in the browser.
- **CozoDB:** SQLite/RocksDB/sled backends. It reports SQLite writes as "1000× slower", but that was on a slow SSD with per-write transactions; tiramemsu's measured 206 µs assert is about 5 k tx/s.
- **Jena TDB2:** append-only CoW B+trees, with old blocks kept until `compact`.

Sources: <https://fjall-rs.github.io/post/fjall-3/>, <https://github.com/cberner/redb> (benchmark table), <https://docs.rs/rocksdb/latest/rocksdb/struct.Transaction.html>, <https://docs.rs/heed/latest/heed/struct.Env.html>, <https://github.com/surrealdb/surrealkv/pull/405>, <https://slatedb.io/>, <https://docs.turso.tech/tursodb/concurrent-writes>, <https://docs.cozodb.org/en/latest/releases/v0.3.html>.

## 3. Formats and designs for "log + indexes"

| Design | How history and time are indexed | Size per fact | Lesson for tiramemsu |
|---|---|---|---|
| **Datomic** | Tx log plus covering EAVT/AEVT/AVET/VAET as immutable wide trees of zipped segments (1k–20k datoms per segment); in-memory novelty merged at a threshold; current and history parts | ~2.5–50 B per datom per index, compressed | log + covering indexes + novelty tail is the proven shape |
| **XTDB v2** | Arrow files in a hash-trie LSM; from L1 split into current and historical, history sharded by "recency"; as-of prunes files by recency | not published | separate current from history at compaction; prune segments by time |
| **TerminusDB** (`terminus-store`) | Stack of succinct delta layers (additions/removals per commit), rollup | ~13.6 B/triple (vendor) | time travel per commit only; repo dormant since 2024 |
| **HDT / HDT-FoQ** | static dictionary + bitmap triples | ~10–11 B/triple, all patterns | static: build once |
| **Ring** (TODS 2024) | one BWT ring serves all 6 orders, worst-case-optimal joins | 6.7–12.7 B/triple | research prototype, static |
| **Permuted tries with Elias-Fano** (Perego/Pibiri/Venturini) | trie per permutation, EF-coded levels | ~6–7 B/triple, all patterns | best compact encoding for sealed permutation segments |
| **Kafka segments** | `.log` plus sparse `.index` (8 B every 4 KB) and `.timeindex` (12 B entries) | ~0 index overhead | a time→position map needs a sparse index, not a B-tree per row |
| **Couchstore / Hyperbee** | append-only B-trees: every commit appends a new root path | grows until compaction | append-only B-tree = write amplification paid in space |
| **OSTRICH** (versioned RDF) | HDT snapshot plus a delta chain with 6 B+trees (add/del × SPO/POS/OSP) | BEAR-B hourly: 187 MB vs 24 MB HDT | delta chains are slow to ingest (4 497 s); avoid |
| **Parquet / Lance / Vortex** | column chunks + zone maps + bloom | very small | scans only; Vortex/Arrow encodings usable *inside* a segment; Parquet is poor for point lookups |
| **Roaring bitmaps** | set of retracted eids | ~2 B per element sparse, ~1 bit dense | as-of visibility as a bitmap checkpoint plus a replay of the log tail |

Also relevant: *Disk-Based Interval Indexes Under the Increasing Ending Time Assumption* (arXiv 2606.22773, 2026 *(unverified, not read)*), which matches `t_ret` arriving in order.

Sources: <https://tonsky.me/blog/unofficial-guide-to-datomic-internals/>, <https://xtdb.com/blog/building-a-bitemp-index-3-storage>, <https://terminusdb.com/blog/terminusdb-internals/>, <https://arxiv.org/abs/1904.07619>, <https://doi.org/10.1145/3644824>, <https://cwiki.apache.org/confluence/display/KAFKA/KIP-33+-+Add+a+time+based+log+index>, <https://rdfostrich.github.io/article-jws2018-ostrich/>.

## 4. Three options, from cheapest to most radical

### A. Stay on SQLite, with an "append-aware" format 2 (recommended next step)

This keeps D26, the executor tiers, WASM/DO hosts, triggers, savepoints and SQL codegen, and goes after the 5.3× index overhead directly.

1. **Drop `log_add` (9.9 MB of 168.6, ~6 %).**
   - Add `tx.first_eid`, a sparse index in the Kafka style: one entry per transaction, not per row.
   - "Added by `t`" becomes `eid < :e_next(t)`, a rowid range.
   - `event` "since t" becomes a rowid range scan of `triple`.
   - This holds because a single writer allocates eids in commit order, burned ids included.
   - **Caveat:** merged files with foreign origins (D35) break this monotonicity. Merging would need a per-origin `first_eid`.
2. **Take `t_add` out of the `hist_*` keys.** Every SQLite index entry already ends in the rowid, which is the eid, and the eid orders like `t_add`. The as-of predicate `t_add <= :t` becomes `eid < :e`. This shortens every history entry by a varint. The CozoDB newest-first seek is lost only within one `(s,p,o)`, where versions are few. Gate on the as-of churn benchmark.
3. **Make `hist_*` partial on `t_ret IS NOT NULL`.** This is already a candidate in `storage.md#Measured Footprint` (about −30 %), with an as-of read of live `UNION ALL` dead.
4. **Keep the in-place `t_ret` update.** A separate append-only `retraction(eid, t_ret, kind)` table would make rows truly immutable. But the live partial indexes depend on `t_ret` in the row, and "now" would turn into an anti-join. Not worth it.

Expected result: roughly 90–110 B/statement *(estimate)*, the same engine, and a format migration that only drops and rebuilds indexes, so Never Forget is respected.

### B. A native KV backend behind the `Store` interface

The Lean `ReadStore`/`WriteStore` classes are engine-neutral, so a second implementation is possible.

**An LSM is the natural match for the append-only shape:**
- An assert appends to seven keyspaces: `spo`/`pos`/`osp` live and history, plus `valid_p`.
- A retract is an append too: a tombstone in the three live keyspaces and a `(t_ret, eid)` key in `log_ret`.
- Compaction drops live tombstones, which is how "current state skips dead rows" (Fluree/XTDB) comes for free.
- The values are empty, keys are big-endian packed `u64`s, and v3 prefix truncation compresses shared `(s, p)` prefixes.

**Engine choice:**
- **RocksDB** meets the whole contract: column families, stackable savepoints, snapshots, prefix bloom.
- **fjall** is pure Rust, but needs a savepoint overlay you write yourself.
- **heed/LMDB** gives the fastest reads and real nested transactions, but it is a CoW B-tree, so it is "SQLite without SQL" with little size win.

**Cost:** you lose SQL codegen (tm-exec would need native join operators; M4 LFTJ points that way), the in-file triggers, FTS5, WASM and Durable Objects. CozoDB's numbers suggest large write-throughput gains. Point reads are not faster than SQLite's 4 µs: redb's table shows RocksDB and fjall reading slower than LMDB.

**Justified only if** measured write throughput becomes the bottleneck. That is the open "scale ceiling" input in `overview.md#Open Inputs`.

### C. Custom sealed segments, Datomic/XTDB-style (research only)

- **Novelty tail.** A mutable tail (the current SQLite file, or a memtable) holds recent transactions.
- **Sealed segments.** Each eid range is periodically sealed into an immutable segment. A segment holds:
  - column-encoded facts (`s, p, o, v_from, v_to` as FoR/delta blocks; `t_add` implied by eid ranges);
  - `spo`/`pos`/`osp` permutation tries with Elias-Fano levels (~6–7 B/triple);
  - a roaring bitmap of eids retracted by the segment's watermark, plus the append-only `(t_ret, eid)` log for retractions after it.
- **As-of(t).** Skip segments with first eid ≥ `E(t)`, and test retraction with the bitmap checkpoint plus a replay of the log tail.
- **Expected size.** ~20–35 B/statement *(estimate)*, and worst-case-optimal joins over tries come almost for free.
- **Cost.** A storage engine to build and verify: merge, crash recovery, compaction. At the D1 scale of 10⁴–10⁷ statements, the whole store is 1.5 GB or less on SQLite. Option C pays off only beyond that scale, which `overview.md#Non-Goals` excludes.

## 5. Recommendation

1. **Do A now**, behind the existing benchmarks (`size`, `churn`, `as-of`, oxilite `as-of-latency`). It is the only option that keeps every host and decision, and it attacks the measured problem, which is index overhead, not the engine.
2. **Keep B as the planned escape hatch.** Write a property-based refinement test of the `Store` interface against a fjall- or RocksDB-backed prototype only if a write-throughput target appears. RocksDB if C++ is acceptable, fjall if pure Rust matters more than savepoints.
3. **Borrow C's ideas without building it:**
   - the eid↔tx sparse map (used in A);
   - roaring retraction checkpoints, if as-of over deep history ever regresses;
   - EF-coded tries, as the in-memory structure for an M4 LFTJ operator.
4. **Rejected:**
   - DuckDB/Parquet as the primary store (D26);
   - Bitcask (no range scans);
   - TerminusDB-style delta chains and OSTRICH (per-commit granularity, slow ingest);
   - sled, nebari and Speedb (unmaintained);
   - SlateDB (object-store latency).
