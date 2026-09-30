//! Host glue: a host-neutral [`TableFunction`] exposed as an eponymous-only
//! virtual table, so SQL can call it as `name(arg, …)` in a FROM clause.
//!
//! This is the only module of the workspace with `unsafe` code: rusqlite's
//! virtual-table traits are `unsafe` to implement because SQLite reads the
//! `#[repr(C)]` base structs directly. No raw pointer is dereferenced here.
#![allow(unsafe_code)]

use std::borrow::Cow;
use std::ffi::{c_int, CStr, CString};
use std::marker::PhantomData;
use std::sync::Arc;

use rusqlite::types::Value as RValue;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
    VTabCursor,
};
use rusqlite::{ffi, Connection};
use tm_core::{SqlValue, TableFunction};

/// Registers `f` as an eponymous-only virtual table on `conn`.
pub fn register(conn: &Connection, f: TableFunction) -> rusqlite::Result<()> {
    const MODULE: Module<'static, FnTab> = Module::eponymous_only_module();
    let name = f.name.clone();
    conn.create_module(name.as_str(), &MODULE, Some(Arc::new(f)))
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// The virtual table of one table function.
#[repr(C)]
struct FnTab {
    /// Base class. Must be first.
    base: ffi::sqlite3_vtab,
    f: Arc<TableFunction>,
}

unsafe impl<'vtab> VTab<'vtab> for FnTab {
    type Aux = Arc<TableFunction>;
    type Cursor = FnCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<TableFunction>>,
        _module_name: &[u8],
        _database_name: &[u8],
        _table_name: &[u8],
        _args: &[&[u8]],
    ) -> rusqlite::Result<(Cow<'static, CStr>, FnTab)> {
        let f = aux
            .cloned()
            .ok_or_else(|| rusqlite::Error::ModuleError("table function without body".into()))?;
        db.config(VTabConfig::Innocuous)?;
        let mut cols: Vec<String> = f.columns.iter().map(|c| quote(c)).collect();
        cols.extend(
            f.args
                .iter()
                .map(|a| format!("{} HIDDEN", quote(&format!("arg_{a}")))),
        );
        let schema = CString::new(format!("CREATE TABLE x({})", cols.join(", ")))
            .map_err(|e| rusqlite::Error::ModuleError(e.to_string()))?;
        Ok((
            Cow::Owned(schema),
            FnTab {
                base: ffi::sqlite3_vtab::default(),
                f,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> rusqlite::Result<bool> {
        let n_cols = self.f.columns.len() as c_int;
        let n_args = self.f.args.len();
        let mut slot: Vec<Option<usize>> = vec![None; n_args];
        let mut unusable = false;
        for (i, c) in info.constraints().enumerate() {
            let col = c.column();
            if col < n_cols {
                continue;
            }
            let a = (col - n_cols) as usize;
            if a >= n_args {
                continue;
            }
            if !c.is_usable() {
                if slot[a].is_none() {
                    unusable = true;
                }
                continue;
            }
            if c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ {
                slot[a] = Some(i);
            }
        }
        // an argument that is supplied but not yet usable (a correlated column of a
        // table not joined yet) makes this plan impossible
        if unusable && slot.iter().any(Option::is_none) {
            return Ok(false);
        }
        let mut mask = 0;
        let mut argv = 0;
        for (a, s) in slot.iter().enumerate() {
            if let Some(i) = s {
                argv += 1;
                mask |= 1 << a;
                let mut u = info.constraint_usage(*i);
                u.set_argv_index(argv);
                u.set_omit(true);
            }
        }
        info.set_idx_num(mask);
        info.set_estimated_cost(10.0);
        info.set_estimated_rows(10);
        Ok(true)
    }

    fn open(&'vtab mut self) -> rusqlite::Result<FnCursor<'vtab>> {
        Ok(FnCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            f: self.f.clone(),
            rows: Vec::new(),
            pos: 0,
            phantom: PhantomData,
        })
    }
}

/// A cursor over the materialised rows of one call.
#[repr(C)]
struct FnCursor<'vtab> {
    /// Base class. Must be first.
    base: ffi::sqlite3_vtab_cursor,
    f: Arc<TableFunction>,
    rows: Vec<Vec<SqlValue>>,
    pos: usize,
    phantom: PhantomData<&'vtab FnTab>,
}

fn from_rvalue(v: RValue) -> SqlValue {
    match v {
        RValue::Null => SqlValue::Null,
        RValue::Integer(i) => SqlValue::Integer(i),
        RValue::Real(r) => SqlValue::Real(r),
        RValue::Text(t) => SqlValue::Text(t),
        RValue::Blob(b) => SqlValue::Blob(b),
    }
}

fn to_rvalue(v: &SqlValue) -> RValue {
    match v {
        SqlValue::Null => RValue::Null,
        SqlValue::Integer(i) => RValue::Integer(*i),
        SqlValue::Real(r) => RValue::Real(*r),
        SqlValue::Text(s) => RValue::Text(s.clone()),
        SqlValue::Blob(b) => RValue::Blob(b.clone()),
    }
}

unsafe impl VTabCursor for FnCursor<'_> {
    fn filter(
        &mut self,
        idx_num: c_int,
        _idx_str: Option<&str>,
        args: &Filters<'_>,
    ) -> rusqlite::Result<()> {
        let mut vals = Vec::with_capacity(self.f.args.len());
        let mut k = 0;
        for a in 0..self.f.args.len() {
            if idx_num & (1 << a) != 0 {
                vals.push(from_rvalue(args.get::<RValue>(k)?));
                k += 1;
            } else {
                vals.push(SqlValue::Null);
            }
        }
        self.rows = (self.f.func)(&vals).map_err(rusqlite::Error::ModuleError)?;
        self.pos = 0;
        Ok(())
    }

    fn next(&mut self) -> rusqlite::Result<()> {
        self.pos += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.pos >= self.rows.len()
    }

    fn column(&self, ctx: &mut Context, i: c_int) -> rusqlite::Result<()> {
        let v = self
            .rows
            .get(self.pos)
            .and_then(|r| r.get(i as usize))
            .map_or(RValue::Null, to_rvalue);
        ctx.set_result(&v)
    }

    fn rowid(&self) -> rusqlite::Result<i64> {
        Ok(self.pos as i64)
    }
}
