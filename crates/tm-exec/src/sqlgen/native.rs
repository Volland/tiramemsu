//! Native regions as FROM items: `tm_path(start, path, mode, max_hops, view) AS
//! pN` with a correlated or bound start (design D7).

use tm_core::{Result, SqlValue};

use super::{Col, Gen, Item, Rel};
use crate::plan::analyze::Dom;
use crate::plan::{PPath, PTerm};

impl Gen<'_> {
    /// Compiles a path pattern; its start is a constant or a column of `acc`.
    pub fn path(&mut self, p: &PPath, acc: &Rel) -> Result<Rel> {
        let start = match &p.arg {
            PTerm::Id(id) => self.params.id(*id),
            PTerm::Var(v) => match acc.col(v) {
                Some(c) => c.sql.clone(),
                None => {
                    return self.unsupported(
                        "a path whose bound endpoint is not bound by a pattern joined with it",
                    )
                }
            },
        };
        let a = self.alias('p');
        let path = self.params.push(SqlValue::Text(p.text.clone()));
        let mode = self.params.push(SqlValue::Text(p.mode.to_string()));
        let max = self.params.push(
            p.max_hops
                .map_or(SqlValue::Null, |m| SqlValue::Integer(m as i64)),
        );
        let view = self.params.push(SqlValue::Text(p.view_text.clone()));
        self.path_region(&a, p.note);
        let mut r = Rel {
            items: vec![Item {
                sql: format!("tm_path({start}, {path}, {mode}, {max}, {view}) AS {a}"),
                on: None,
            }],
            ..Rel::default()
        };
        match &p.other {
            PTerm::Id(id) => {
                let ph = self.params.id(*id);
                r.conds.push(format!("{a}.\"end\" = {ph}"));
            }
            PTerm::Var(v) => r.set(v, Col::term(format!("{a}.\"end\""), false)),
        }
        if let Some(b) = &p.bind_path {
            r.set(
                b,
                Col {
                    sql: format!("{a}.path_json"),
                    dom: Dom::PathJson,
                    mm: false,
                    eid_of: None,
                },
            );
        }
        Ok(r)
    }
}
