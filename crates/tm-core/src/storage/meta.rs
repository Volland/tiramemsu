//! Engine metadata counters kept in the `meta` table.

use crate::error::{Error, Result};
use crate::exec::{Executor, SqlValue};

/// Every counter the engine keeps in `meta` (plus `format_version`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Next term-dictionary id.
    pub next_term: i64,
    /// Next `NODE` payload.
    pub next_node: i64,
    /// Next `BNODE` payload.
    pub next_bnode: i64,
    /// Next statement number.
    pub next_stmt: i64,
    /// Last committed transaction number.
    pub last_t: i64,
    /// Instant of the last committed transaction.
    pub last_instant: i64,
    /// Version of `pred_multi`; increases whenever a predicate is added.
    pub multi_version: i64,
}

/// The `meta` rows of a fresh database, in insertion order.
pub const INITIAL: [(&str, i64); 8] = [
    ("format_version", 1),
    ("next_term", 1),
    ("next_node", 1),
    ("next_bnode", 1),
    ("next_stmt", 1),
    ("last_t", 0),
    ("last_instant", 0),
    ("multi_version", 0),
];

impl Counters {
    /// Reads the counters from `meta`.
    pub fn load(exec: &mut dyn Executor) -> Result<Counters> {
        let mut c = Counters::default();
        let mut seen = 0;
        exec.query("SELECT key, value FROM meta", &[], &mut |r| {
            let v = r[1].as_i64().unwrap_or(0);
            let slot = match r[0].as_str() {
                Some("next_term") => &mut c.next_term,
                Some("next_node") => &mut c.next_node,
                Some("next_bnode") => &mut c.next_bnode,
                Some("next_stmt") => &mut c.next_stmt,
                Some("last_t") => &mut c.last_t,
                Some("last_instant") => &mut c.last_instant,
                Some("multi_version") => &mut c.multi_version,
                _ => return Ok(()),
            };
            *slot = v;
            seen += 1;
            Ok(())
        })?;
        if seen < 7 {
            return Err(Error::InvalidTerm {
                position: crate::error::Position::Value,
                reason: "meta table is missing counters".to_string(),
            });
        }
        Ok(c)
    }

    fn pairs(&self) -> [(&'static str, i64); 7] {
        [
            ("next_term", self.next_term),
            ("next_node", self.next_node),
            ("next_bnode", self.next_bnode),
            ("next_stmt", self.next_stmt),
            ("last_t", self.last_t),
            ("last_instant", self.last_instant),
            ("multi_version", self.multi_version),
        ]
    }

    /// Writes every counter that differs from `before`.
    pub fn store(&self, before: &Counters, exec: &mut dyn Executor) -> Result<()> {
        for ((k, v), (_, old)) in self.pairs().into_iter().zip(before.pairs()) {
            if v != old {
                exec.execute(
                    "UPDATE meta SET value = ?1 WHERE key = ?2",
                    &[SqlValue::Integer(v), SqlValue::from(k)],
                )?;
            }
        }
        Ok(())
    }

    /// The id counters only (`last_t`, `last_instant` and `multi_version` taken from `base`).
    pub fn ids_over(&self, base: &Counters) -> Counters {
        Counters {
            next_term: self.next_term,
            next_node: self.next_node,
            next_bnode: self.next_bnode,
            next_stmt: self.next_stmt,
            ..*base
        }
    }
}
