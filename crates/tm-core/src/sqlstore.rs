//! The storage traits over a SQL [`Executor`]: a SQLite file read and written as a
//! [`ReadStore`] / [`WriteStore`].
//!
//! Lossless conversion between backends (`tiramemsu::convert`) copies through
//! these traits. The statements are the ones the SQL engine issues for the same
//! purpose. The text index is FTS5 here and is rebuilt after a copy, so the text
//! operations of the traits are no-ops on this store.

use std::cell::RefCell;

use crate::error::{Error, Result};
use crate::exec::{Executor, SqlValue};
use crate::report::RetKind;
use crate::store::{
    EmbeddingRow, Pattern, ReadStore, RowIter, Scope, StmtRow, TermRow, TextDoc, VolatileRow,
    WriteStore,
};
use crate::term::TermKey;

const COLS: &str = "eid, s, p, o, t_add, t_ret, v_from, v_to, ret_kind";

fn row_of(r: &[SqlValue]) -> StmtRow {
    let i = |k: usize| r[k].as_i64();
    StmtRow {
        eid: i(0).unwrap_or(0),
        s: i(1).unwrap_or(0),
        p: i(2).unwrap_or(0),
        o: i(3).unwrap_or(0),
        t_add: i(4).unwrap_or(0),
        t_ret: i(5),
        v_from: i(6),
        v_to: i(7),
        ret_kind: i(8),
    }
}

/// The connection of a [`SqlStore`]: borrowed from a caller, or owned by a
/// transaction of [`SqlBackend`].
enum Conn<'a> {
    Borrowed(&'a mut dyn Executor),
    Owned(Box<dyn Executor>),
    /// A finished backend transaction: its connection went back to the backend.
    Gone,
}

impl Conn<'_> {
    fn get(&mut self) -> Result<&mut dyn Executor> {
        match self {
            Conn::Borrowed(e) => Ok(&mut **e),
            Conn::Owned(e) => Ok(e.as_mut()),
            Conn::Gone => Err(Error::storage("the SQLite transaction already ended")),
        }
    }
}

/// Where a backend transaction's connection goes when it ends.
type Slot = std::sync::Arc<std::sync::Mutex<Option<Box<dyn Executor>>>>;

/// A SQLite connection in an open transaction, seen through the storage traits.
pub struct SqlStore<'a> {
    exec: RefCell<Conn<'a>>,
    /// Set for a transaction of [`SqlBackend`]: the slot its connection returns to.
    home: Option<Slot>,
}

impl<'a> SqlStore<'a> {
    /// Wraps a connection; the caller owns the transaction around it.
    pub fn new(exec: &'a mut dyn Executor) -> SqlStore<'a> {
        SqlStore {
            exec: RefCell::new(Conn::Borrowed(exec)),
            home: None,
        }
    }

    /// Ends a backend transaction (commit or rollback) and returns its
    /// connection to the backend; a no-op on a borrowed connection.
    fn finish(&mut self, commit: bool) -> Result<()> {
        let Some(home) = self.home.take() else {
            return Ok(());
        };
        let conn = std::mem::replace(self.exec.get_mut(), Conn::Gone);
        let Conn::Owned(mut exec) = conn else {
            return Ok(());
        };
        let r = if commit {
            exec.commit()
        } else {
            exec.rollback()
        };
        if let Ok(mut slot) = home.lock() {
            *slot = Some(exec);
        }
        r
    }

    fn rows(&self, sql: &str, params: &[SqlValue]) -> Result<Vec<Vec<SqlValue>>> {
        self.exec
            .try_borrow_mut()
            .map_err(|_| Error::Reentrant)?
            .get()?
            .rows(sql, params)
    }

    fn one(&self, sql: &str, params: &[SqlValue]) -> Result<Option<i64>> {
        self.exec
            .try_borrow_mut()
            .map_err(|_| Error::Reentrant)?
            .get()?
            .query_i64(sql, params)
    }

    fn execute(&self, sql: &str, params: &[SqlValue]) -> Result<usize> {
        self.exec
            .try_borrow_mut()
            .map_err(|_| Error::Reentrant)?
            .get()?
            .execute(sql, params)
    }

    fn stmts(&self, sql: &str, params: &[SqlValue]) -> Result<RowIter<'_>> {
        let rows: Vec<Result<StmtRow>> = self
            .rows(sql, params)?
            .iter()
            .map(|r| Ok(row_of(r)))
            .collect();
        Ok(Box::new(rows.into_iter()))
    }
}

impl ReadStore for SqlStore<'_> {
    fn meta(&self, key: &str) -> Result<Option<i64>> {
        self.one(
            "SELECT value FROM meta WHERE key = ?1",
            &[SqlValue::from(key)],
        )
    }

    fn term(&self, id: i64) -> Result<Option<TermRow>> {
        let mut exec = self.exec.try_borrow_mut().map_err(|_| Error::Reentrant)?;
        Ok(crate::term::load_term(exec.get()?, id)?.map(|t| TermRow {
            tag: t.tag as u8,
            lex: t.lex,
            dt: t.dt.map(|d| d.raw()),
            lang: t.lang,
            num: t.num,
        }))
    }

    fn term_id(&self, k: &TermKey) -> Result<Option<i64>> {
        self.one(
            "SELECT id FROM term WHERE tag = +?1 AND lex = +?2 \
             AND ifnull(dt, 0) = ifnull(+?3, 0) AND ifnull(lang, '') = ifnull(+?4, '')",
            &[
                SqlValue::Integer(i64::from(k.tag)),
                SqlValue::Text(k.lex.clone()),
                SqlValue::from(k.dt),
                k.lang.clone().map_or(SqlValue::Null, SqlValue::Text),
            ],
        )
    }

    fn tx_instant(&self, t: i64) -> Result<Option<i64>> {
        self.one(
            "SELECT instant FROM tx WHERE t = ?1",
            &[SqlValue::Integer(t)],
        )
    }

    fn tx_at_or_before(&self, instant: i64) -> Result<i64> {
        Ok(self
            .one(
                "SELECT coalesce(max(t), 0) FROM tx WHERE instant <= ?1",
                &[SqlValue::Integer(instant)],
            )?
            .unwrap_or(0))
    }

    fn statement(&self, eid: i64) -> Result<Option<StmtRow>> {
        Ok(self
            .rows(
                &format!("SELECT {COLS} FROM triple WHERE eid = ?1"),
                &[SqlValue::Integer(eid)],
            )?
            .first()
            .map(|r| row_of(r)))
    }

    fn scan(&self, pat: Pattern, scope: Scope) -> Result<RowIter<'_>> {
        let mut conds = Vec::new();
        let mut params = Vec::new();
        for (col, v) in [("s", pat.s), ("p", pat.p), ("o", pat.o)] {
            if let Some(v) = v {
                params.push(SqlValue::Integer(v));
                conds.push(format!("{col} = +?{}", params.len()));
            }
        }
        if scope == Scope::Live {
            conds.push("t_ret IS NULL".to_string());
        }
        let where_ = if conds.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conds.join(" AND "))
        };
        self.stmts(&format!("SELECT {COLS} FROM triple{where_}"), &params)
    }

    fn added_since(&self, t: i64) -> Result<RowIter<'_>> {
        self.stmts(
            &format!("SELECT {COLS} FROM triple WHERE t_add > ?1 ORDER BY t_add, eid"),
            &[SqlValue::Integer(t)],
        )
    }

    fn retracted_since(&self, t: i64) -> Result<RowIter<'_>> {
        self.stmts(
            &format!("SELECT {COLS} FROM triple WHERE t_ret > ?1 ORDER BY t_ret, eid"),
            &[SqlValue::Integer(t)],
        )
    }

    fn pred_multi(&self, p: i64) -> Result<bool> {
        Ok(self
            .one(
                "SELECT p FROM pred_multi WHERE p = ?1",
                &[SqlValue::Integer(p)],
            )?
            .is_some())
    }

    fn pred_multi_all(&self) -> Result<Vec<i64>> {
        Ok(self
            .rows("SELECT p FROM pred_multi ORDER BY p", &[])?
            .iter()
            .filter_map(|r| r[0].as_i64())
            .collect())
    }

    fn volatile(&self, s: i64, key: i64) -> Result<Option<VolatileRow>> {
        Ok(self
            .rows(
                "SELECT value, updated_at FROM volatile WHERE s = ?1 AND key = ?2",
                &[SqlValue::Integer(s), SqlValue::Integer(key)],
            )?
            .first()
            .map(|r| VolatileRow {
                value: r[0].as_i64().unwrap_or(0),
                updated_at: r[1].as_i64().unwrap_or(0),
            }))
    }

    fn volatile_all(&self) -> Result<Vec<(i64, i64, VolatileRow)>> {
        Ok(self
            .rows(
                "SELECT s, key, value, updated_at FROM volatile ORDER BY s, key",
                &[],
            )?
            .iter()
            .map(|r| {
                let i = |k: usize| r[k].as_i64().unwrap_or(0);
                (
                    i(0),
                    i(1),
                    VolatileRow {
                        value: i(2),
                        updated_at: i(3),
                    },
                )
            })
            .collect())
    }

    fn embedding(&self, model: i64, eid: i64) -> Result<Option<EmbeddingRow>> {
        Ok(self
            .rows(
                "SELECT model, eid, t, bits, vec FROM embedding WHERE model = ?1 AND eid = ?2",
                &[SqlValue::Integer(model), SqlValue::Integer(eid)],
            )?
            .first()
            .map(|r| embedding_of(r)))
    }

    fn embeddings(
        &self,
        model: i64,
    ) -> Result<Box<dyn Iterator<Item = Result<EmbeddingRow>> + '_>> {
        let rows: Vec<Result<EmbeddingRow>> = self
            .rows(
                "SELECT model, eid, t, bits, vec FROM embedding WHERE model = ?1 ORDER BY eid",
                &[SqlValue::Integer(model)],
            )?
            .iter()
            .map(|r| Ok(embedding_of(r)))
            .collect();
        Ok(Box::new(rows.into_iter()))
    }

    fn embeddings_all(&self) -> Result<Vec<EmbeddingRow>> {
        Ok(self
            .rows(
                "SELECT model, eid, t, bits, vec FROM embedding ORDER BY model, eid",
                &[],
            )?
            .iter()
            .map(|r| embedding_of(r))
            .collect())
    }

    fn text_doc(&self, _value: i64) -> Result<Option<TextDoc>> {
        Ok(None)
    }

    fn text_postings(&self, _token: &str) -> Result<Vec<(i64, Vec<u32>)>> {
        Ok(Vec::new())
    }

    fn text_tokens_with_prefix(&self, _prefix: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn text_stats(&self) -> Result<(u64, u64)> {
        Ok((0, 0))
    }
}

fn embedding_of(r: &[SqlValue]) -> EmbeddingRow {
    EmbeddingRow {
        model: r[0].as_i64().unwrap_or(0),
        eid: r[1].as_i64().unwrap_or(0),
        t: r[2].as_i64().unwrap_or(0),
        bits: r[3].as_blob().unwrap_or_default().to_vec(),
        vec: r[4].as_blob().unwrap_or_default().to_vec(),
    }
}

impl WriteStore for SqlStore<'_> {
    fn set_meta(&mut self, key: &str, value: i64) -> Result<()> {
        self.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            &[SqlValue::from(key), SqlValue::Integer(value)],
        )?;
        Ok(())
    }

    fn insert_term(&mut self, id: i64, row: &TermRow) -> Result<()> {
        self.execute(
            "INSERT INTO term(id, tag, lex, dt, lang, num) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            &[
                SqlValue::Integer(id),
                SqlValue::Integer(i64::from(row.tag)),
                SqlValue::Text(row.lex.clone()),
                SqlValue::from(row.dt),
                row.lang.clone().map_or(SqlValue::Null, SqlValue::Text),
                SqlValue::from(row.num),
            ],
        )?;
        Ok(())
    }

    fn insert_tx(&mut self, t: i64, instant: i64) -> Result<()> {
        self.execute(
            "INSERT INTO tx(t, instant) VALUES (?1, ?2)",
            &[SqlValue::Integer(t), SqlValue::Integer(instant)],
        )?;
        Ok(())
    }

    fn insert_statement(&mut self, r: &StmtRow) -> Result<()> {
        if !r.live() {
            return Err(Error::storage("a statement is inserted live"));
        }
        self.execute(
            "INSERT INTO triple(eid, s, p, o, t_add, v_from, v_to) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            &[
                SqlValue::Integer(r.eid),
                SqlValue::Integer(r.s),
                SqlValue::Integer(r.p),
                SqlValue::Integer(r.o),
                SqlValue::Integer(r.t_add),
                SqlValue::from(r.v_from),
                SqlValue::from(r.v_to),
            ],
        )?;
        Ok(())
    }

    fn retract(&mut self, eid: i64, t: i64, kind: RetKind) -> Result<bool> {
        Ok(self.execute(
            "UPDATE triple SET t_ret = ?1, ret_kind = ?2 WHERE eid = ?3 AND t_ret IS NULL",
            &[
                SqlValue::Integer(t),
                SqlValue::Integer(kind as i64),
                SqlValue::Integer(eid),
            ],
        )? > 0)
    }

    fn add_pred_multi(&mut self, p: i64) -> Result<bool> {
        Ok(self.execute(
            "INSERT INTO pred_multi(p) SELECT ?1 WHERE NOT EXISTS (SELECT 1 FROM pred_multi WHERE p = ?1)",
            &[SqlValue::Integer(p)],
        )? > 0)
    }

    fn put_volatile(&mut self, s: i64, key: i64, row: VolatileRow) -> Result<()> {
        self.execute(
            "INSERT INTO volatile(s, key, value, updated_at) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(s, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            &[
                SqlValue::Integer(s),
                SqlValue::Integer(key),
                SqlValue::Integer(row.value),
                SqlValue::Integer(row.updated_at),
            ],
        )?;
        Ok(())
    }

    fn delete_volatile(&mut self, s: i64, key: i64) -> Result<bool> {
        Ok(self.execute(
            "DELETE FROM volatile WHERE s = ?1 AND key = ?2",
            &[SqlValue::Integer(s), SqlValue::Integer(key)],
        )? > 0)
    }

    fn insert_embedding(&mut self, row: &EmbeddingRow) -> Result<()> {
        self.execute(
            "INSERT INTO embedding(model, eid, t, bits, vec) VALUES (?1, ?2, ?3, ?4, ?5)",
            &[
                SqlValue::Integer(row.model),
                SqlValue::Integer(row.eid),
                SqlValue::Integer(row.t),
                SqlValue::Blob(row.bits.clone()),
                SqlValue::Blob(row.vec.clone()),
            ],
        )?;
        Ok(())
    }

    fn text_insert(
        &mut self,
        _value: i64,
        _doc: &TextDoc,
        _tokens: &[(String, Vec<u32>)],
    ) -> Result<bool> {
        Ok(false)
    }

    fn text_clear(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Drop for SqlStore<'_> {
    fn drop(&mut self) {
        // an unfinished backend transaction rolls back
        let _ = self.finish(false);
    }
}

impl crate::store::WriteTxn for SqlStore<'static> {
    fn commit(mut self: Box<Self>) -> Result<()> {
        self.finish(true)
    }

    fn abort(mut self: Box<Self>) -> Result<()> {
        self.finish(false)
    }
}

// @lat: [[backends#One Engine In Two Phases]]
/// A SQLite file as a storage [`Backend`](crate::store::Backend) for the trait
/// engine (`tm_store::Engine`): one connection, taken by each read or write
/// transaction and given back when it ends, so reads and writes take turns.
pub struct SqlBackend {
    slot: Slot,
}

impl SqlBackend {
    /// A backend over an opened and initialised writer connection
    /// (`storage::open`).
    pub fn new(exec: Box<dyn Executor>) -> SqlBackend {
        SqlBackend {
            slot: std::sync::Arc::new(std::sync::Mutex::new(Some(exec))),
        }
    }

    fn begin(&self, write: bool) -> Result<SqlStore<'static>> {
        let mut exec = self
            .slot
            .lock()
            .map_err(|_| Error::storage("SQLite backend lock poisoned"))?
            .take()
            .ok_or(Error::Reentrant)?;
        let r = if write {
            exec.begin_immediate()
        } else {
            exec.begin_read()
        };
        if let Err(e) = r {
            if let Ok(mut slot) = self.slot.lock() {
                *slot = Some(exec);
            }
            return Err(e);
        }
        Ok(SqlStore {
            exec: RefCell::new(Conn::Owned(exec)),
            home: Some(self.slot.clone()),
        })
    }
}

impl crate::store::Backend for SqlBackend {
    fn name(&self) -> &'static str {
        "sqlite"
    }

    fn read(&self) -> Result<Box<dyn ReadStore + Send>> {
        Ok(Box::new(self.begin(false)?))
    }

    fn write(&self) -> Result<Box<dyn crate::store::WriteTxn>> {
        Ok(Box::new(self.begin(true)?))
    }
}
