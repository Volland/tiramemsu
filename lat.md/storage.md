# Storage

One SQLite file in WAL mode holds the term dictionary, transactions, the triple table with its indexes, volatile state and engine metadata. All tables are STRICT.

SQLite maintains every permutation index, so the engine has no code to keep them in sync by hand. Current-state indexes are partial (`WHERE t_ret IS NULL`), which keeps "now" queries off dead rows. See [[prior-art#CozoDB]] and [[prior-art#Fluree]] for where the idea came from.

## Schema

The complete DDL for format version 1. It is checked: it loads on SQLite 3.53, and the query plans below use covering indexes.

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;

CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value INTEGER NOT NULL
) STRICT;
-- keys: format_version, next_term, next_node, next_bnode, next_stmt, last_t, last_instant, multi_version

CREATE TABLE term (
  id   INTEGER PRIMARY KEY,   -- payload; ObjectId = id << 4 | tag
  tag  INTEGER NOT NULL,      -- IRI, STR, LANG_STR, TYPED, DOUBLE, DECIMAL
  lex  TEXT NOT NULL,
  dt   INTEGER,               -- datatype IRI ObjectId (TYPED / DOUBLE / DECIMAL)
  lang TEXT,                  -- LANG_STR only, lower-cased
  num  REAL                   -- DOUBLE / DECIMAL numeric value
) STRICT;
-- NULL-safe: SQLite UNIQUE treats NULLs as distinct, so dt/lang are coalesced
CREATE UNIQUE INDEX term_key ON term(tag, lex, ifnull(dt, 0), ifnull(lang, ''));
CREATE INDEX term_num ON term(num) WHERE num IS NOT NULL;

CREATE TABLE tx (
  t       INTEGER PRIMARY KEY,
  instant INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX tx_instant ON tx(instant);

CREATE TABLE triple (
  eid      INTEGER PRIMARY KEY,  -- STMT ObjectId
  s        INTEGER NOT NULL,
  p        INTEGER NOT NULL,
  o        INTEGER NOT NULL,
  t_add    INTEGER NOT NULL,
  t_ret    INTEGER,              -- NULL = live
  v_from   INTEGER,              -- NULL = unbounded
  v_to     INTEGER,              -- NULL = unbounded
  ret_kind INTEGER               -- 0 explicit, 1 cascade, 2 supersede, 3 cardinality
) STRICT;

-- current state (partial, covering)
CREATE INDEX live_spo ON triple(s, p, o, t_ret, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX live_pos ON triple(p, o, s, t_ret, v_from, v_to) WHERE t_ret IS NULL;
CREATE INDEX live_osp ON triple(o, s, p, t_ret, v_from, v_to) WHERE t_ret IS NULL;
-- time travel (full, newest first)
CREATE INDEX hist_spo ON triple(s, p, o, t_add DESC, t_ret, v_from, v_to);
CREATE INDEX hist_pos ON triple(p, o, s, t_add DESC, t_ret, v_from, v_to);
CREATE INDEX hist_osp ON triple(o, s, p, t_add DESC, t_ret, v_from, v_to);
-- valid time over current beliefs
CREATE INDEX valid_p ON triple(p, v_from, v_to) WHERE t_ret IS NULL;
-- the log
CREATE INDEX log_add ON triple(t_add);
CREATE INDEX log_ret ON triple(t_ret, ret_kind) WHERE t_ret IS NOT NULL;

CREATE TABLE volatile (
  s          INTEGER NOT NULL,
  key        INTEGER NOT NULL,
  value      INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (s, key)
) WITHOUT ROWID, STRICT;

-- predicates that ever held two eids with the same (s, p, o); only grows
CREATE TABLE pred_multi (
  p INTEGER PRIMARY KEY          -- predicate ObjectId
) STRICT;

-- planner statistics: SQLite's own sqlite_stat1 / sqlite_stat4, created by
-- PRAGMA optimize at open, plus a full ANALYZE when STAT4 samples are missing (see query.md#Join Ordering)
PRAGMA optimize = 0x10002;
```

Format 1 reserves names for later milestones, so that no user migration clashes with them: tag 15 `SEALED` and the table `seal_key` (M6, [[time-model#Erasure]]), and the tables `term_fts` and `vec_*` (M7, [[roadmap#Milestones]]). Format 2 adds the `meta` rows `text_index` and `text_stale` of [[storage#Text Index]]; `term_fts` is created only when that index is built.

## Triple Table

One row per statement occurrence. The row is its own lifetime: `t_add` is the assert, and `t_ret` (with `ret_kind`) is the retract. Since eids are never reused, each eid has exactly one interval.

- `eid` is the rowid and holds the full `STMT` ObjectId. Eids are allocated from `meta.next_stmt`, never from `max(rowid)`.
- **Counter bound:** `NODE`, `BNODE`, `STMT` and `TX` numbers stop at 2⁴⁸ − 1, so `meta.next_*` reaches at most 2⁴⁸ and `last_t` at most 2⁴⁸ − 1. Allocating past the bound fails the transaction with `IdSpaceExhausted { kind }`; the high payload bits are the reserved origin ([[data-model#ObjectId#Origin Bits]]).
- **Burned ids:** after a speculative `with` or a `dry_run` rolls back, the writer re-applies the advanced `meta` counters in a small commit. An id that was ever shown to a caller is then never issued again, even though its triple never existed. See [[time-model#Speculative Transactions]].
- The valid-time columns are in every index key so that views combining time filters stay covering.
- A row is only ever updated from `t_ret IS NULL` to a value. See [[storage#Invariant Triggers]].

## Graph Memberships

A graph membership `(e, sys:inGraph, g)` is an ordinary row of the triple table, so it costs one row and nine index entries, and `GRAPH <g>` seeks `(p, o)` on the existing indexes. See [[data-model#Named Graphs]].

On the benchmark fixture (`bench/named-graphs`, 1.1 M statements, 700 000 live), giving every live statement one membership doubled the live rows and grew the file 1.92 times (rows 1.64 times, because retracted rows get none). A graph of 100 members seeks in microseconds and one of 500 000 members takes 44 ms. On the 11 M-statement fixture the ratio is the same (1.92 times, 3.38 GB against 1.76 GB), a 100-member graph takes 0.01 ms, a 50 000-member graph 49 ms and a 5 M-member graph 550 ms (the same join without a graph: 102 ms). No format change is needed. SQLite picks `hist_pos` or `live_pos` for the `(p, o)` seek at equal cost; both are seeks, and after churn plus `ANALYZE` it takes `live_pos`.

## Multi-Eid Predicates

`pred_multi` lists every predicate that has ever held two eids with the same `(s, p, o)`, live or retracted. SPARQL removes duplicates only for those predicates. See [[query#Front Ends#SPARQL]].

- The writer adds `p` when an insert finds another row with the same `(s, p, o)` in `hist_spo`, which is the lookup assert already makes. That happens through `create`, through a second valid-time episode, or through re-asserting after a retract, which the History view shows twice.
- A predicate is never removed from the list. That keeps the list safe for every view, including `asOf` and History. It is bookkeeping, not graph data, so [[time-model#Never Forget]] does not cover it.
- `meta.multi_version` increases whenever a predicate is added. Cached SQL for SPARQL queries is keyed by it, so no cached plan skips duplicate removal for a predicate that now needs it.
- Measured on 750 000 triples, a set-semantics 2-hop join took 10.3 µs with duplicate removal and 5.0 µs without it.

## Term Dictionary

Values that do not fit inline (IRIs, long strings, language strings, other datatypes, doubles, decimals) live in `term`, keyed by `(tag, lex, dt, lang)`. Terms are never deleted.

- Lookup and insert happen in the writer transaction: `SELECT id FROM term WHERE tag=? AND lex=? AND ifnull(dt,0)=ifnull(?,0) AND ifnull(lang,'')=ifnull(?,'')` (this uses `term_key`), and if it is missing, insert with `meta.next_term`.
- `term_key` is an expression index, because a plain UNIQUE index treats NULL `dt`/`lang` as distinct and would let duplicate IRIs and strings through.
- `num` carries the numeric value of `DOUBLE` and `DECIMAL`, and `term_num` serves range filters on them. See [[data-model#ObjectId#Range Scans]].
- The reader side keeps an LRU cache of `id → term`, since result decoding is the hot path.

## Query Shapes

The view-aware scan emits exactly these predicate shapes. The partial index is chosen only when `t_ret IS NULL` appears verbatim.

```sql
-- now
SELECT o, eid FROM triple WHERE s=:s AND p=:p AND t_ret IS NULL;
-- asOf t
SELECT o, eid FROM triple WHERE s=:s AND p=:p
  AND t_add <= :t AND (t_ret IS NULL OR t_ret > :t);
-- validAt d (combine with either of the above)
  AND (v_from IS NULL OR v_from <= :d) AND (v_to IS NULL OR v_to > :d)
-- history: no time predicate
```

Verified plans (SQLite 3.53): "now" uses `COVERING INDEX live_spo`, "asOf" uses `COVERING INDEX hist_spo`, and "valid on date" over current beliefs uses `valid_p`. See [[query#Views and Scans]].

- `t_ret` stays in the `live_*` keys even though it is always NULL there. Without it, SQLite 3.53 no longer treats the live index as covering for `t_ret IS NULL`, and picks `hist_*` instead.
- In multi-pattern joins, SQLite may still pick `hist_*` for a `Now` pattern whose predicate has no retracted rows, because the cost is equal. See [[query#Physical Planning#Join Ordering]].

## Measured Footprint

With every index in the schema, 1.1 million statements (700 000 live, 400 000 retracted) take 168.6 MB after `VACUUM`: about 153 bytes per statement, with the indexes at 5.3× the table.

Measured on SQLite 3.53 with small ids, so varints stay 1–3 bytes. By B-tree: `triple` 26.6 MB, each `hist_*` 25.0 MB, each `live_*` 14.6 MB, `log_add` 9.9 MB, `valid_p` 8.4 MB, `log_ret` 4.8 MB.

- The three `hist_*` indexes are 44 % of the file, and they repeat every live row that `live_*` already holds.
- **Candidate, benchmark-gated:** make `hist_*` partial on `t_ret IS NOT NULL` and read as-of as a live branch `UNION ALL` a dead branch. On this data that saves about 30 %. The cost is that SQLite does not flatten a compound subquery into a join, so every as-of pattern would become a subquery. It is adopted only if the as-of benchmarks show no regression. See [[roadmap#Benchmarks]].
- oxilite writes about 4.8 rows per triple (index entries included) without history and 8.8 with its as-of index ([[prior-art#oxilite]]). The comparable figure here is 9 B-tree entries per assert. A retract moves the row out of the live indexes, rewrites its three `hist_*` entries and adds a `log_ret` entry.

## Event View

The event log is a view, not a table, because every event is already recorded in `t_add` and `t_ret`.

```sql
CREATE VIEW event(t, eid, op, kind) AS
  SELECT t_add, eid, 'assert', NULL FROM triple
  UNION ALL
  SELECT t_ret, eid, 'retract', ret_kind FROM triple WHERE t_ret IS NOT NULL;
```

`since t` is `SELECT … FROM event WHERE t > :t ORDER BY t`. It uses `log_add` and `log_ret`. See [[time-model#Event Log]].

## Invariant Triggers

Triggers make "never forget" a property of the file itself, not just of the engine code. Any tool that opens the file with plain SQLite gets the same protection.

```sql
CREATE TRIGGER triple_no_delete BEFORE DELETE ON triple
BEGIN SELECT RAISE(ABORT, 'tiramemsu: triples are never deleted'); END;

CREATE TRIGGER triple_retract_once BEFORE UPDATE ON triple
WHEN OLD.t_ret IS NOT NULL
  OR NEW.eid IS NOT OLD.eid OR NEW.s IS NOT OLD.s OR NEW.p IS NOT OLD.p
  OR NEW.o IS NOT OLD.o OR NEW.t_add IS NOT OLD.t_add
  OR NEW.v_from IS NOT OLD.v_from OR NEW.v_to IS NOT OLD.v_to
  OR NEW.t_ret IS NULL
BEGIN SELECT RAISE(ABORT, 'tiramemsu: only a single retraction is allowed'); END;

-- INSERT OR REPLACE deletes the old row without firing DELETE triggers
-- (unless recursive_triggers is on), so re-inserting an existing key is blocked explicitly.
CREATE TRIGGER triple_no_replace BEFORE INSERT ON triple
WHEN EXISTS (SELECT 1 FROM triple WHERE eid = NEW.eid)
BEGIN SELECT RAISE(ABORT, 'tiramemsu: eids are never reused'); END;
CREATE TRIGGER term_no_replace BEFORE INSERT ON term
WHEN EXISTS (SELECT 1 FROM term WHERE id = NEW.id)
BEGIN SELECT RAISE(ABORT, 'tiramemsu: term ids are never reused'); END;
CREATE TRIGGER tx_no_replace BEFORE INSERT ON tx
WHEN EXISTS (SELECT 1 FROM tx WHERE t = NEW.t)
BEGIN SELECT RAISE(ABORT, 'tiramemsu: transaction numbers are never reused'); END;

CREATE TRIGGER term_no_delete BEFORE DELETE ON term
BEGIN SELECT RAISE(ABORT, 'tiramemsu: terms are never deleted'); END;
CREATE TRIGGER term_no_update BEFORE UPDATE ON term
BEGIN SELECT RAISE(ABORT, 'tiramemsu: terms are immutable'); END;

CREATE TRIGGER tx_no_delete BEFORE DELETE ON tx
BEGIN SELECT RAISE(ABORT, 'tiramemsu: transactions are never deleted'); END;
CREATE TRIGGER tx_no_update BEFORE UPDATE ON tx
BEGIN SELECT RAISE(ABORT, 'tiramemsu: transactions are immutable'); END;
```

`ROLLBACK TO` a savepoint is not a DELETE, so speculative transactions still work. See [[time-model#Speculative Transactions]].

## Volatile Table

High-churn state (`lastSeen`, counters, per-turn scores) lives in `volatile(s, key, value, updated_at)`. It is outside the graph: no eid, no history, no layers, and ordinary upserts are allowed.

- Queries see volatile values as virtual properties, e.g. Cypher `n.lastSeen`. A key present in both places resolves to the triple.
- The rule of thumb: if you would ever ask "why" or "as of when" about a value, it is a triple. Otherwise it is volatile.
- Volatile values do not time-travel. Under `asOf` or `history` views, volatile properties are absent rather than wrong.
- Writes are part of their transaction: `set_volatile` upserts with `updated_at` = the tx instant, `clear_volatile` deletes the row. A failed transaction, a dry run or a speculation discards them.
- Before a query language exists, `View::values(s, key)` resolves a key: statement objects first, else the volatile value under `Now` only.

## Format Versioning

`meta.format_version` records the schema version. Opening a file with an unknown newer version fails, and older versions are migrated forward inside one transaction.

Migrations must respect [[time-model#Never Forget]]. They may add columns, indexes and tables, but never drop or rewrite triples.

- **Format 3** is the current version. The migrations are [[crates/tm-core/src/storage/migrate.rs#MIGRATIONS]].
- **Format 2** inserts the `meta` rows `text_index = 0` and `text_stale = 0` of [[storage#Text Index]]. It runs on every host, creates no FTS5 table and touches no `triple`, `term` or `tx` row.
- **Format 3** creates the empty derived tables of [[storage#Saved Answers]] and reads or writes no graph row.
- A new file is created as format 1 and migrated forward like an old file, so fresh and migrated files have the same schema.
- The bump is what keeps the derived index honest: a format-1 build, which would write strings without indexing them, refuses a format-2 file with `FormatVersion` instead of letting `term_fts` drift.

## Text Index

The derived FTS5 index behind text recall ([[query#Text Recall]]). It holds one row per distinct string ever stored as a statement object, and can always be rebuilt from the graph.

```sql
CREATE VIRTUAL TABLE term_fts USING fts5(text, lang UNINDEXED,
  tokenize = 'unicode61 remove_diacritics 2');
-- rowid = the value's full ObjectId (SHORT_STR, STR or LANG_STR)
-- meta: text_index = index version (0: never built), text_stale = lowest eid not indexed (0: none)
```

- **What is indexed:** plain strings, inline (`SHORT_STR`, decoded into the index without a dictionary row) or in the dictionary (`STR`), and language-tagged strings (`LANG_STR`, with their tag in `lang`). Typed literals, IRIs and numbers are not searchable. Values of retracted statements stay indexed; visibility is decided at recall time.
- **Opt-in:** a file has no `term_fts` until the index is built by `OpenOptions::text_index`, `Db::enable_text_index` or `Db::rebuild_text_index`. Once built, every writer on a host with FTS5 keeps it current.
- **Upkeep:** each new statement with a string object adds its value inside the write transaction (`Tx::index_text`, through [[crates/tm-core/src/text.rs#index_value]]), so speculations, dry runs and failed transactions roll their index rows back with everything else.
- **Hosts without FTS5** never issue FTS5 SQL. While the index exists they set `text_stale` to the first string statement they write. The next writer with FTS5 (at open or at the start of a transaction) indexes every string statement from that eid and clears it; until then recall fails with `TextIndexUnavailable` rather than miss hits.
- **Rebuild** ([[crates/tm-core/src/text.rs#rebuild]]) drops and refills `term_fts` from the statements in one write transaction. It reads `triple` and `term` and changes neither, nor `tx`; recall afterwards returns what it returned before. `text_index` stores the layout version, so a later tokenizer change is a rebuild, not a format migration.

## Saved Answers

Derived records of saved queries and their last results ([[query#Saved Answers]]). Format 3 creates the two tables; they stay empty until an answer is saved and never feed back into the graph.

```sql
CREATE TABLE saved_answer (
  name TEXT PRIMARY KEY, layout INTEGER NOT NULL,       -- record layout version (1)
  language TEXT NOT NULL, query TEXT NOT NULL,          -- 'sparql' | 'cypher', verbatim text
  params TEXT NOT NULL, view TEXT NOT NULL,             -- JSON, exactly as saved
  settings TEXT NOT NULL,                               -- JSON: @vocab and prefixes at save time
  result TEXT NOT NULL, coverage TEXT NOT NULL,         -- JSON
  checkpoint INTEGER NOT NULL, cursor INTEGER NOT NULL, -- t the result reflects; events processed
  evaluated_at INTEGER NOT NULL, revision INTEGER NOT NULL,
  status INTEGER NOT NULL,                              -- 0 fresh, 1 recheck, 2 stale
  cause INTEGER, cause_t INTEGER, cause_eid INTEGER, cause_op INTEGER, cause_kind INTEGER,
  error TEXT                                            -- last failed refresh
) STRICT;
CREATE TABLE saved_answer_dep (name TEXT NOT NULL, eid INTEGER NOT NULL,
  PRIMARY KEY (name, eid)) STRICT, WITHOUT ROWID;      -- the cited statements
CREATE INDEX saved_answer_dep_eid ON saved_answer_dep(eid);
```

- **Derived, not history:** rows are written by `Store::derived_write`, one `BEGIN IMMEDIATE` that allocates no transaction number, logs no event and changes no counter. No invariant trigger guards them, and deleting a saved answer deletes only its record.
- **Layout version:** `layout` lets a later build change the record shape without a format bump; a build refuses a row of a layout it does not know with `Unsupported`.
- **Why a format bump:** a format-2 build would leave the tables alone and still be correct, since the cursor replays the log, but the rule of [[storage#Format Versioning]] is that every schema addition is a migration.
