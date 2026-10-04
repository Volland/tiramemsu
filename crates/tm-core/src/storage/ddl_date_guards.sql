-- Format 4: dates belong to the latest uncommitted engine transaction.
-- last_t is the committed watermark; the engine advances it on commit.
-- Protection assumes schema and engine metadata are not deliberately tampered with.
CREATE TRIGGER triple_add_date BEFORE INSERT ON triple
WHEN NOT EXISTS (SELECT 1 FROM triple WHERE eid = NEW.eid)
 AND (NEW.t_add <= (SELECT value FROM meta WHERE key = 'last_t')
   OR NEW.t_add IS NOT (SELECT max(t) FROM tx)
   OR (NEW.t_ret IS NOT NULL AND NEW.t_ret IS NOT NEW.t_add))
BEGIN SELECT RAISE(ABORT, 'tiramemsu: assertion must use the active transaction'); END;

CREATE TRIGGER triple_ret_date BEFORE UPDATE OF t_ret ON triple
WHEN OLD.t_ret IS NULL AND NEW.t_ret IS NOT NULL
 AND NEW.eid IS OLD.eid AND NEW.s IS OLD.s AND NEW.p IS OLD.p
 AND NEW.o IS OLD.o AND NEW.t_add IS OLD.t_add
 AND NEW.v_from IS OLD.v_from AND NEW.v_to IS OLD.v_to
 AND (NEW.t_ret <= (SELECT value FROM meta WHERE key = 'last_t')
   OR NEW.t_ret IS NOT (SELECT max(t) FROM tx)
   OR NEW.t_ret < OLD.t_add)
BEGIN SELECT RAISE(ABORT, 'tiramemsu: retraction must use the active transaction'); END;
