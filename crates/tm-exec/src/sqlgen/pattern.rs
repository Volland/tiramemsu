//! Triple patterns: one `triple` alias each, constants and repeated variables as
//! equalities, the view predicates, the canonical-eid predicate for set semantics
//! (design D10), virtual predicates (D6) and volatile values.

use tm_core::{Result, SqlValue};
use tm_ir::MatchMode;

use super::{Col, Gen, IsoPat, Item, Rel};
use crate::plan::{PObj, PTerm, PTriple, PVirtual, PVolatile};
use crate::scan::{view_predicates, ResolvedView};

impl Gen<'_> {
    /// `triple AS tN` with its constraints.
    pub fn triple(&mut self, t: &PTriple) -> Result<Rel> {
        let a = self.alias('t');
        self.record_triple_alias(&a);
        let mut r = Rel {
            items: vec![Item {
                sql: format!("triple AS {a}"),
                on: None,
            }],
            ..Rel::default()
        };
        for (col, pos) in [("s", &t.s), ("p", &t.p), ("o", &t.o)] {
            match pos {
                PTerm::Id(id) => {
                    let ph = self.params.id(*id);
                    r.conds.push(format!("{a}.{col} = {ph}"));
                }
                PTerm::Var(v) => match r.col(v) {
                    Some(c) => {
                        let prev = c.sql.clone();
                        r.conds.push(format!("{a}.{col} = {prev}"));
                    }
                    None => r.set(v, Col::term(format!("{a}.{col}"), false)),
                },
            }
        }
        if let Some(e) = &t.eid {
            match r.col(e) {
                Some(c) => {
                    let prev = c.sql.clone();
                    r.conds.push(format!("{a}.eid = {prev}"));
                }
                None => r.set(
                    e,
                    Col {
                        eid_of: Some((a.clone(), t.view)),
                        ..Col::term(format!("{a}.eid"), false)
                    },
                ),
            }
        }
        r.conds
            .extend(view_predicates(&a, &t.view, &mut self.params));
        if t.canonical {
            let x = self.alias('x');
            let mut w = vec![
                format!("{x}.s = {a}.s"),
                format!("{x}.p = {a}.p"),
                format!("{x}.o = {a}.o"),
            ];
            w.extend(view_predicates(&x, &t.view, &mut self.params));
            r.conds.push(format!(
                "{a}.eid = (SELECT min({x}.eid) FROM triple AS {x} WHERE {})",
                w.join(" AND ")
            ));
        }
        if self.sem.match_mode == MatchMode::RelIsomorphism {
            if let Some(g) = t.iso_group {
                r.iso.push(IsoPat {
                    group: g,
                    eid: format!("{a}.eid"),
                    pred: match t.p {
                        PTerm::Id(id) => Some(id),
                        PTerm::Var(_) => None,
                    },
                    optional: false,
                });
            }
        }
        Ok(r)
    }

    /// A virtual-predicate pattern. With `reuse`, the columns are read from that
    /// alias (the eid pattern of the subject, under an identical view) and no
    /// FROM item is added.
    pub fn virtual_pattern(&mut self, v: &PVirtual, reuse: Option<&str>) -> Result<Rel> {
        let mut r = Rel::default();
        let a = match reuse {
            Some(a) => a.to_string(),
            None => {
                let a = self.alias('t');
                self.record_triple_alias(&a);
                r.items.push(Item {
                    sql: format!("triple AS {a}"),
                    on: None,
                });
                match &v.subject {
                    PTerm::Id(id) => {
                        let ph = self.params.id(*id);
                        r.conds.push(format!("{a}.eid = {ph}"));
                    }
                    PTerm::Var(s) => r.set(
                        s,
                        Col {
                            eid_of: Some((a.clone(), v.view)),
                            ..Col::term(format!("{a}.eid"), false)
                        },
                    ),
                }
                r.conds
                    .extend(view_predicates(&a, &v.view, &mut self.params));
                a
            }
        };
        if let Some(nn) = v.pred.not_null(&a) {
            r.conds.push(nn);
        }
        match &v.object {
            PObj::Column(c) => {
                let ph = self.params.push(SqlValue::Integer(*c));
                r.conds.push(v.pred.object_compare(&a, &ph));
            }
            PObj::Var(o) => {
                let expr = v.pred.object_expr(&a);
                match r.col(o) {
                    Some(c) => {
                        let prev = c.sql.clone();
                        r.conds.push(format!("{expr} = {prev}"));
                    }
                    None => r.set(o, Col::term(expr, false)),
                }
            }
        }
        Ok(r)
    }

    /// A pattern that also matches volatile values: stored live triples, plus the
    /// volatile rows of `(s, key)` pairs that have no live triple.
    pub fn volatile(&mut self, v: &PVolatile) -> Result<Rel> {
        let t = self.alias('t');
        self.record_triple_alias(&t);
        let vv = self.alias('v');
        let x = self.alias('x');
        let k = self.params.id(v.p);
        let mut tw = vec![format!("{t}.p = {k}")];
        tw.extend(view_predicates(&t, &ResolvedView::NOW, &mut self.params));
        let mut vw = vec![format!("{vv}.key = {k}")];
        for (col, vcol, pos) in [("s", "s", &v.s), ("o", "value", &v.o)] {
            if let PTerm::Id(id) = pos {
                let ph = self.params.id(*id);
                tw.push(format!("{t}.{col} = {ph}"));
                vw.push(format!("{vv}.{vcol} = {ph}"));
            }
        }
        let mut xw = vec![format!("{x}.s = {vv}.s"), format!("{x}.p = {vv}.key")];
        xw.extend(view_predicates(&x, &ResolvedView::NOW, &mut self.params));
        vw.push(format!(
            "NOT EXISTS (SELECT 1 FROM triple AS {x} WHERE {})",
            xw.join(" AND ")
        ));
        let q = self.alias('q');
        let sql = format!(
            "(SELECT {t}.s AS s, {t}.o AS o FROM triple AS {t} WHERE {} UNION ALL \
             SELECT {vv}.s, {vv}.value FROM volatile AS {vv} WHERE {}) AS {q}",
            tw.join(" AND "),
            vw.join(" AND ")
        );
        let mut r = Rel {
            items: vec![Item { sql, on: None }],
            ..Rel::default()
        };
        for (col, pos) in [("s", &v.s), ("o", &v.o)] {
            if let PTerm::Var(var) = pos {
                match r.col(var) {
                    Some(c) => {
                        let prev = c.sql.clone();
                        r.conds.push(format!("{q}.{col} = {prev}"));
                    }
                    None => r.set(var, Col::term(format!("{q}.{col}"), false)),
                }
            }
        }
        Ok(r)
    }
}
