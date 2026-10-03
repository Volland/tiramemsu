-- Format 3: saved answers (OpenSpec change add-saved-answer-invalidation).
-- Verbatim DDL of lat.md/storage.md#Saved Answers. These are derived records:
-- they never take a transaction number, never appear in the event log, and no
-- invariant trigger guards them. Graph rows (triple, term, tx) are untouched.

CREATE TABLE saved_answer (
  name         TEXT PRIMARY KEY,
  layout       INTEGER NOT NULL,  -- record layout version (1)
  language     TEXT NOT NULL,     -- 'sparql' | 'cypher'
  query        TEXT NOT NULL,     -- the query text, verbatim
  params       TEXT NOT NULL,     -- JSON: the query parameters
  view         TEXT NOT NULL,     -- JSON: the view descriptor as given
  settings     TEXT NOT NULL,     -- JSON: @vocab and prefixes at save time
  result       TEXT NOT NULL,     -- JSON: the last successful result
  coverage     TEXT NOT NULL,     -- JSON: coverage reason names
  checkpoint   INTEGER NOT NULL,  -- last_t the result reflects
  cursor       INTEGER NOT NULL,  -- events processed through this t
  evaluated_at INTEGER NOT NULL,  -- clock ms of the last successful evaluation
  revision     INTEGER NOT NULL,  -- successful evaluations so far
  status       INTEGER NOT NULL,  -- 0 fresh, 1 recheck, 2 stale
  cause        INTEGER,           -- why it is not fresh (NULL when fresh)
  cause_t      INTEGER,           -- the transaction of the triggering event
  cause_eid    INTEGER,           -- the statement of the triggering event
  cause_op     INTEGER,           -- 0 assert, 1 retract
  cause_kind   INTEGER,           -- ret_kind of a retract event
  error        TEXT               -- the last failed refresh, until one succeeds
) STRICT;

-- the statements the last result cited (SPARQL provenance)
CREATE TABLE saved_answer_dep (
  name TEXT NOT NULL,
  eid  INTEGER NOT NULL,          -- STMT ObjectId, as triple.eid
  PRIMARY KEY (name, eid)
) STRICT, WITHOUT ROWID;
CREATE INDEX saved_answer_dep_eid ON saved_answer_dep(eid);
