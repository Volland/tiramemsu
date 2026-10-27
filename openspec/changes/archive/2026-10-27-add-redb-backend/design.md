## Context

`add-store-trait` gives one engine over `ReadStore`/`WriteStore`. This change implements those traits on redb 4.4 and plugs the result into the facade. redb offers: typed tables with ordered keys; one write transaction at a time and any number of MVCC read transactions, each a stable snapshot; read cursors (`lower_bound`, `next`, `peek_next`); per-transaction durability (`None`, `Immediate`); ephemeral savepoints only on a clean transaction; `compact()`; a 9-byte magic header `redb\x1A\x0A\xA9\x0D\x0A`.

## Decisions

### D-1. Same integers, sorted bytes

Every id is stored as the same signed 64-bit raw the SQLite file holds. Keys are fixed-width byte arrays of big-endian words with the sign bit flipped (`(x as u64) ^ 1<<63`), so redb's byte order equals numeric order and a key prefix is a byte prefix. A custom `Key` type compares with one `memcmp` on 24, 32 or 40 bytes.

- motivated_by:: [[openspec:redb-storage#Same logical database]] and the requirement that `convert` is lossless.

### D-2. Six live permutations, three history permutations

| Table | Key (32 or 40 bytes) | Value |
|---|---|---|
| `l_spo`, `l_sop`, `l_pso`, `l_pos`, `l_osp`, `l_ops` | `(a, b, c, eid)` | `v_from, v_to` (16 bytes, `i64::MIN`/`MAX` for unbounded) |
| `h_spo`, `h_pos`, `h_osp` | `(a, b, c, !t_add, eid)` | `t_ret` (0 = live), `ret_kind`, `v_from`, `v_to` |
| `stmt` | `eid` | full record |
| `log_add` / `log_ret` | `(t, eid)` | `ret_kind` (log_ret) |

Every subset of bound positions is a key prefix of some live table, in every variable order a multiway join can choose. A retraction deletes six live keys and rewrites three history values and the `stmt` record. Valid time sits in the value, so live scans need no second lookup.

Rejected: three live permutations (some join orders would need a sort or a hash build; see design interview Q3), six history permutations (writes and size grow for the rarer `asOf` joins), keeping valid time in the key (wider keys for a filter that is rarely selective).

- motivated_by:: [[query#Physical Planning#LFTJ]] (sorted prefixes in every order), [[prior-art#MillenniumDB]].

### D-3. Exact statistics in the write transaction

`pred_stats[p] = (live, distinct_s, distinct_o)` is updated on insert and retract. Distinct counts change only when the `(p, s)` or `(p, o)` prefix becomes empty or non-empty, which one `l_pso`/`l_pos` probe of two keys decides. `count_hint` uses these for one bound position and a bounded cursor walk (≤ 4 096 keys, exact below) for two.

### D-4. Snapshot readers, no pool

A `View` on redb is a `ReadTransaction` held for the view's lifetime. There is no reader pool and no `readers` limit; `OpenOptions::readers` and `reader_timeout` are ignored on redb (documented). The writer is a mutex around `begin_write`, as on SQLite.

### D-5. Text index as redb tables

`text_tok[(token, value_id)] = term frequency`, `text_doc[value_id] = (length, lang)`, `text_stats = (docs, total_len)`. Tokenisation reproduces FTS5 `unicode61 remove_diacritics 2`: Unicode alphanumeric runs, simple case folding, NFD and removal of combining marks. Scoring is FTS5's `bm25()` with k1 = 1.2 and b = 0.75 and FTS5's IDF formula. Index rows are written in the statement's transaction, so speculation and failures roll them back, as on SQLite.

Rejected: tantivy (separate segment files and commits, so the index could disagree with the graph after a crash and could not roll back with a speculation).

### D-6. Durability

`Durability::Normal` commits with redb `Durability::None` and makes the database durable from a background flusher at most every 50 ms or 1 000 commits (`OpenOptions::flush_interval`, `flush_commits`), and on `Db::sync()` and close. `Immediate` commits every transaction with `Durability::Immediate`. `None` never flushes in the background. SQLite maps them to `synchronous = NORMAL`, `FULL` and `OFF`. After a crash a redb file returns to its last durable commit: consistent, possibly missing the newest transactions, which is SQLite NORMAL's promise.

### D-7. Detection and creation

`Db::open` reads up to 16 bytes: `SQLite format 3\0` → SQLite, the redb magic → redb, an empty or missing file → create with the requested backend (`Auto` → SQLite), anything else → `UnknownFormat`. A redb file must also carry `meta.engine = "tiramemsu"` and a supported `meta.format_version`, else `ForeignFile` / `FormatVersion`, like SQLite files.

### D-8. Conversion

`convert` copies rows in key order: terms, transactions, statements (with their final retraction), `pred_multi`, volatile, embeddings, counters (including burned ids), then rebuilds the destination's derived structures (permutations or indexes, statistics, text index). It refuses an existing destination.

## Constraints

- **redb savepoints** are refused once a transaction is dirty: the engine never takes one after a write (add-store-trait D-5). Constrains [[openspec:redb-storage#Same logical database]] {strength: must}.
- **Forbidden unsafe code:** all of this is safe Rust; the custom key type compares slices.
- **Platforms:** redb needs a file system; WASM keeps SQLite.

## Risks / Trade-offs

- **Write amplification:** about 11 B-tree inserts per assert (stmt, 6 live, 3 history, log). Measured, not hidden, in `add-backend-benchmarks`.
- **File growth:** copy-on-write pages and fragmentation; `Db::compact()` and `tm compact` reclaim it.
- **Text ranking drift:** BM25 floating-point differences against FTS5; the parity test compares hit sets exactly and ranks with a 1e-9 relative tolerance on scores.

## Resolution

Implemented 2026-10-27.

- **Byte keys.** Every table uses `&[u8]` keys (redb compares them with `memcmp` and shortens internal-node separators); values are fixed `[u8; N]` arrays or byte strings. The integer encoding is design D-1's.
- **Readers.** A `RedbRead` opens every table once per snapshot; writers open a table per operation, so a write transaction never borrows from itself. Write-side scans materialise their (small) result.
- **The storage traits moved into `tm-core`** (`tm_core::store`, re-exported by `tm-store`) so that `tm-core` could define the backend-neutral write handle `TxHandle` and its object-safe `TxOps`. The facade's `Tx` is now `TxHandle`; SPARQL Update and Cypher writes take it. `TxHandle::read_with` (SQL) and `TxHandle::read_store` (storage traits) reach the backend.
- **Conversion** copies through the traits: `tm_core::sqlstore::SqlStore` implements them over a SQL connection, and `tm_store::copy::copy` replays history in transaction order so the SQLite date guards accept it.
- **Compaction** holds the redb `Database` behind an `RwLock` taken shortly to begin transactions and exclusively to compact.
- **Not yet on redb:** saved answers (their tables are SQL) and every query (SPARQL, Cypher, `execute_ir`, paths), which return `MissingCapability("native_runner")` until `add-native-runner`.
- **Evidence.** The `tm-store` oracle runs on redb (300 random histories in CI-sized runs, 1 000 locally) and compares text, vector and hybrid recall with FTS5 (scores within 1e-9); a changed BM25 constant fails it at once.
