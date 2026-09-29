-- Tiramemsu format version 1.
-- Verbatim DDL of lat.md/storage.md#Schema, #Event View and #Invariant Triggers.
-- The PRAGMAs of the documented schema (journal_mode, synchronous, optimize) are
-- issued by storage::open outside the initialisation transaction.

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
-- PRAGMA optimize at open (see query.md#Join Ordering)

CREATE VIEW event(t, eid, op, kind) AS
  SELECT t_add, eid, 'assert', NULL FROM triple
  UNION ALL
  SELECT t_ret, eid, 'retract', ret_kind FROM triple WHERE t_ret IS NOT NULL;

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
