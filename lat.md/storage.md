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
-- keys: format_version, next_term, next_node, next_bnode, next_stmt, last_t, last_instant

CREATE TABLE term (
  id   INTEGER PRIMARY KEY,   -- payload; ObjectId = id << 4 | tag
  tag  INTEGER NOT NULL,      -- IRI, STR, LANG_STR, TYPED, DOUBLE, DECIMAL
  lex  TEXT NOT NULL,
  dt   INTEGER,               -- datatype IRI ObjectId (TYPED / DOUBLE / DECIMAL)
  lang TEXT,                  -- LANG_STR only, lower-cased
  num  REAL                   -- DOUBLE / DECIMAL numeric value
) STRICT;
CREATE UNIQUE INDEX term_key ON term(tag, lex, dt, lang);
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
```

## Triple Table

One row per statement occurrence. The row is its own lifetime: `t_add` is the assert, and `t_ret` (with `ret_kind`) is the retract. Since eids are never reused, each eid has exactly one interval.

- `eid` is the rowid and holds the full `STMT` ObjectId. Eids are allocated from `meta.next_stmt`, never from `max(rowid)`.
- **Burned ids:** after a speculative `with` or a `dry_run` rolls back, the writer re-applies the advanced `meta` counters in a small commit. An id that was ever shown to a caller is then never issued again, even though its triple never existed. See [[time-model#Speculative Transactions]].
- The valid-time columns are in every index key so that views combining time filters stay covering.
- A row is only ever updated from `t_ret IS NULL` to a value. See [[storage#Invariant Triggers]].

## Term Dictionary

Values that do not fit inline (IRIs, long strings, language strings, other datatypes, doubles, decimals) live in `term`, keyed by `(tag, lex, dt, lang)`. Terms are never deleted.

- Lookup and insert happen in the writer transaction: `SELECT id … WHERE tag=? AND lex=? …`, and if it is missing, insert with `meta.next_term`.
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

## Format Versioning

`meta.format_version` records the schema version. Opening a file with an unknown newer version fails, and older versions are migrated forward inside one transaction.

Migrations must respect [[time-model#Never Forget]]. They may add columns, indexes and tables, but never drop or rewrite triples.
