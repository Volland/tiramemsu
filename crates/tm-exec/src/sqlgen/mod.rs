//! SQL generation (design D8): every operator compiles to a [`Rel`] — FROM
//! items, WHERE conjuncts and a column per variable — and the root becomes one
//! SQL statement.
//!
//! Aliases are allocated in order of compilation: `t0…` for triple patterns,
//! `d0…` for term-dictionary lookups, `p0…` for table-valued functions, `q0…` for
//! derived tables, `x0…` for correlated subqueries and `v0…` for the volatile
//! table. Parameters are numbered `?1…` in order of first use; output columns are
//! `c0…cn`, so variable names and data never reach the SQL text.

pub mod expr;
pub mod native;
pub mod ops;
pub mod order;
pub mod pattern;

use tm_core::{ObjectId, Result, SqlValue};
use tm_ir::{Semantics, Var};

use crate::error::unsupported;
use crate::native::{OperatorRegistry, PlannerOptions};
use crate::plan::analyze::{Dom, VClass};
use crate::plan::Node;
use crate::result::{RegionInfo, RegionKind, RouteNote};
use crate::scan::ResolvedView;

/// Positional parameters of the statement being generated.
#[derive(Clone, Debug, Default)]
pub struct ParamAlloc {
    p: tm_core::Params,
}

impl ParamAlloc {
    /// Binds a value and returns its placeholder (`?N`).
    pub fn push(&mut self, v: SqlValue) -> String {
        self.p.push(v)
    }

    /// Binds an ObjectId.
    pub fn id(&mut self, id: ObjectId) -> String {
        self.p.push(SqlValue::Integer(id.raw()))
    }

    /// The core parameter list (for [`tm_core::scan_predicates`]).
    pub fn core(&mut self) -> &mut tm_core::Params {
        &mut self.p
    }

    /// The bound values in `?N` order.
    pub fn values(&self) -> &[SqlValue] {
        self.p.values()
    }
}

/// The SQL expression of one variable in a relation.
#[derive(Clone, Debug, PartialEq)]
pub struct Col {
    /// SQL text.
    pub sql: String,
    /// Static domain.
    pub dom: Dom,
    /// May be NULL (missing).
    pub mm: bool,
    /// The triple alias and view whose `eid` this column is (virtual-predicate
    /// alias reuse).
    pub eid_of: Option<(String, ResolvedView)>,
}

impl Col {
    /// A term column.
    pub fn term(sql: impl Into<String>, mm: bool) -> Col {
        Col {
            sql: sql.into(),
            dom: Dom::Term,
            mm,
            eid_of: None,
        }
    }

    /// An always-missing column.
    pub fn null() -> Col {
        Col::term("NULL", true)
    }
}

/// A FROM item: inner (comma-joined) or `LEFT JOIN … ON`.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// `triple AS t0`, `(SELECT …) AS q0`, `tm_path(…) AS p0`, …
    pub sql: String,
    /// `Some(conds)` for a LEFT JOIN with its ON conjuncts.
    pub on: Option<Vec<String>>,
}

/// A relationship pattern under isomorphism.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoPat {
    /// Match group.
    pub group: u32,
    /// Its triple alias.
    pub alias: String,
    /// Its constant predicate.
    pub pred: Option<ObjectId>,
    /// On the optional side of a LEFT JOIN.
    pub optional: bool,
}

/// A compiled relation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rel {
    /// FROM items.
    pub items: Vec<Item>,
    /// WHERE conjuncts.
    pub conds: Vec<String>,
    /// Columns in order of first binding.
    pub cols: Vec<(Var, Col)>,
    /// Relationship patterns (isomorphism).
    pub iso: Vec<IsoPat>,
}

impl Rel {
    /// The column of `v`.
    pub fn col(&self, v: &Var) -> Option<&Col> {
        self.cols.iter().find(|(x, _)| x == v).map(|(_, c)| c)
    }

    /// Sets (or adds) the column of `v`.
    pub fn set(&mut self, v: &Var, c: Col) {
        match self.cols.iter_mut().find(|(x, _)| x == v) {
            Some(slot) => slot.1 = c,
            None => self.cols.push((v.clone(), c)),
        }
    }
}

/// Clauses of a SELECT built over a relation.
#[derive(Clone, Debug, Default)]
pub struct Select {
    /// The select list: `(expr, alias)`.
    pub list: Vec<(String, String)>,
    /// `SELECT DISTINCT`.
    pub distinct: bool,
    /// GROUP BY expressions.
    pub group_by: Vec<String>,
    /// HAVING conjuncts.
    pub having: Vec<String>,
    /// ORDER BY terms.
    pub order_by: Vec<String>,
    /// LIMIT placeholder.
    pub limit: Option<String>,
    /// OFFSET placeholder.
    pub offset: Option<String>,
}

/// The code generator of one statement.
pub struct Gen<'a> {
    /// Semantic flags of the query.
    pub sem: Semantics,
    /// Bound parameters.
    pub params: ParamAlloc,
    counters: [usize; 6],
    /// Regions found while compiling.
    pub regions: Vec<RegionInfo>,
    bgp_ctx: Option<usize>,
    /// Registered native operators.
    pub reg: &'a OperatorRegistry,
    /// Planner options.
    pub opts: &'a PlannerOptions,
    /// Plan-local terms of constants missing from the dictionary, by ObjectId.
    pub synthetic: Vec<(ObjectId, tm_core::Value)>,
}

impl<'a> Gen<'a> {
    /// A generator for one statement.
    pub fn new(sem: Semantics, reg: &'a OperatorRegistry, opts: &'a PlannerOptions) -> Gen<'a> {
        Gen {
            sem,
            params: ParamAlloc::default(),
            counters: [0; 6],
            regions: Vec::new(),
            bgp_ctx: None,
            reg,
            opts,
            synthetic: Vec::new(),
        }
    }

    /// Makes plan-local terms visible to value operations (sort keys, numbers,
    /// strings) through [`Gen::term_source`].
    pub fn set_synthetic(&mut self, terms: &std::collections::HashMap<i64, tm_core::Value>) {
        let mut v: Vec<(ObjectId, tm_core::Value)> = terms
            .iter()
            .map(|(k, v)| (ObjectId::from_raw(*k), v.clone()))
            .collect();
        v.sort_by_key(|(k, _)| *k);
        self.synthetic = v;
    }

    /// The plan-local id of a value missing from the dictionary, if any.
    pub fn synthetic_id(&self, v: &tm_core::Value) -> Option<ObjectId> {
        self.synthetic.iter().find(|(_, x)| x == v).map(|(k, _)| *k)
    }

    /// The term rows value operations read: `term`, plus the plan-local terms
    /// (bound as parameters) when the query has any.
    pub fn term_source(&mut self) -> String {
        if self.synthetic.is_empty() {
            return "term".to_string();
        }
        let mut rows = Vec::new();
        for (id, v) in self.synthetic.clone() {
            let (lex, lang, num) = match tm_core::codec::encode(&v) {
                tm_core::codec::Encoded::Term(t) => (t.lex, t.lang, t.num),
                tm_core::codec::Encoded::Inline(_) => continue,
            };
            let a = self
                .params
                .push(SqlValue::Integer(id.unsigned_payload() as i64));
            let b = self.params.push(SqlValue::Text(lex));
            let c = self.params.push(num.map_or(SqlValue::Null, SqlValue::Real));
            let d = self
                .params
                .push(lang.map_or(SqlValue::Null, SqlValue::Text));
            rows.push(format!("({a}, {b}, {c}, {d}, NULL)"));
        }
        format!(
            "(SELECT id, lex, num, lang, dt FROM term UNION ALL VALUES {})",
            rows.join(", ")
        )
    }

    /// A fresh alias with prefix `t`, `d`, `p`, `q`, `x` or `v`.
    pub fn alias(&mut self, prefix: char) -> String {
        let i = match prefix {
            't' => 0,
            'd' => 1,
            'p' => 2,
            'q' => 3,
            'x' => 4,
            _ => 5,
        };
        let n = self.counters[i];
        self.counters[i] += 1;
        format!("{prefix}{n}")
    }

    /// Records a triple alias in the current BGP region (or a new one).
    pub(crate) fn record_triple_alias(&mut self, alias: &str) {
        let rid = match self.bgp_ctx {
            Some(r) => r,
            None => self.new_region(RegionKind::Sql, RouteNote::None),
        };
        self.regions[rid].aliases.push(alias.to_string());
    }

    pub(crate) fn new_region(&mut self, kind: RegionKind, note: RouteNote) -> usize {
        self.regions.push(RegionInfo {
            kind,
            note,
            aliases: Vec::new(),
            query_plan: Vec::new(),
        });
        self.regions.len() - 1
    }

    /// A relation holding the unit row.
    pub fn unit(&mut self) -> Rel {
        let q = self.alias('q');
        Rel {
            items: vec![Item {
                sql: format!("(SELECT 1) AS {q}"),
                on: None,
            }],
            ..Rel::default()
        }
    }

    /// Renders the FROM clause of a relation.
    pub fn from_clause(&mut self, rel: &Rel) -> String {
        if rel.items.is_empty() {
            let q = self.alias('q');
            return format!("(SELECT 1) AS {q}");
        }
        let mut s = String::new();
        for (i, it) in rel.items.iter().enumerate() {
            match &it.on {
                None if i == 0 => s.push_str(&it.sql),
                None => {
                    s.push_str(", ");
                    s.push_str(&it.sql);
                }
                Some(on) => {
                    if i == 0 {
                        // cannot happen: a LEFT JOIN always has a left side
                        s.push_str("(SELECT 1) AS lj");
                    }
                    let on = if on.is_empty() {
                        "1".to_string()
                    } else {
                        on.join(" AND ")
                    };
                    s.push_str(&format!(" LEFT JOIN {} ON {on}", it.sql));
                }
            }
        }
        s
    }

    /// Renders a SELECT over `rel`.
    pub fn select(&mut self, rel: &Rel, sel: &Select) -> String {
        let list = if sel.list.is_empty() {
            "1 AS c0".to_string()
        } else {
            sel.list
                .iter()
                .map(|(e, a)| format!("{e} AS {a}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut s = format!(
            "SELECT {}{list} FROM {}",
            if sel.distinct { "DISTINCT " } else { "" },
            self.from_clause(rel)
        );
        if !rel.conds.is_empty() {
            s.push_str(" WHERE ");
            s.push_str(&rel.conds.join(" AND "));
        }
        if !sel.group_by.is_empty() {
            s.push_str(" GROUP BY ");
            s.push_str(&sel.group_by.join(", "));
        }
        if !sel.having.is_empty() {
            s.push_str(" HAVING ");
            s.push_str(&sel.having.join(" AND "));
        }
        if !sel.order_by.is_empty() {
            s.push_str(" ORDER BY ");
            s.push_str(&sel.order_by.join(", "));
        }
        match (&sel.limit, &sel.offset) {
            (Some(l), Some(o)) => s.push_str(&format!(" LIMIT {l} OFFSET {o}")),
            (Some(l), None) => s.push_str(&format!(" LIMIT {l}")),
            (None, Some(o)) => s.push_str(&format!(" LIMIT -1 OFFSET {o}")),
            (None, None) => {}
        }
        s
    }

    /// Wraps `rel` into a derived table exposing its columns as `qN.cK`, plus
    /// `extra` computed columns.
    pub fn derive(&mut self, rel: Rel, mut sel: Select, extra: Vec<(Var, Col)>) -> Rel {
        let mut cols: Vec<(Var, Col)> = Vec::new();
        sel.list.clear();
        for (v, c) in rel.cols.iter().chain(extra.iter()) {
            if cols.iter().any(|(x, _)| x == v) {
                continue;
            }
            let k = format!("c{}", sel.list.len());
            sel.list.push((c.sql.clone(), k.clone()));
            cols.push((v.clone(), c.clone()));
        }
        let body = self.select(&rel, &sel);
        let q = self.alias('q');
        let cols = cols
            .into_iter()
            .enumerate()
            .map(|(i, (v, c))| {
                (
                    v,
                    Col {
                        sql: format!("{q}.c{i}"),
                        dom: c.dom,
                        mm: c.mm,
                        eid_of: None,
                    },
                )
            })
            .collect();
        Rel {
            items: vec![Item {
                sql: format!("({body}) AS {q}"),
                on: None,
            }],
            conds: Vec::new(),
            cols,
            iso: Vec::new(),
        }
    }

    /// Compiles a plan node into a relation.
    pub fn compile(&mut self, node: &Node) -> Result<Rel> {
        match node {
            Node::Triple(t) => self.triple(t),
            Node::Virtual(v) => self.virtual_pattern(v, None),
            Node::Volatile(v) => self.volatile(v),
            Node::Path(p) => {
                let saved = self.bgp_ctx.take();
                let r = self.path(p, &Rel::default());
                self.bgp_ctx = saved;
                r
            }
            _ => {
                let saved = self.bgp_ctx.take();
                let r = self.compile_op(node);
                self.bgp_ctx = saved;
                r
            }
        }
    }

    fn compile_op(&mut self, node: &Node) -> Result<Rel> {
        match node {
            Node::Empty(vars) => {
                let mut r = self.unit();
                r.conds.push("0".to_string());
                for v in vars {
                    r.set(v, Col::null());
                }
                Ok(r)
            }
            Node::Values(v) => self.values(v),
            Node::Unnest(input, list, var) => self.unnest(input, list, var),
            Node::Join(inputs, null_safe) => self.join(inputs, null_safe),
            Node::LeftJoin(l, r, c) => self.left_join(l, r, c.as_ref()),
            Node::Filter(input, cond) => self.filter(input, cond),
            Node::Union(inputs) => self.union(inputs),
            Node::Extend(input, var, e) => {
                let mut r = self.compile(input)?;
                let v = self.value(e, &r)?;
                let c = self.val_col(v);
                r.set(var, c);
                Ok(r)
            }
            Node::Aggregate(input, group, aggs) => self.aggregate(input, group, aggs, None),
            Node::Project(input, vars, distinct) => {
                let r = self.compile(input)?;
                let mut out = Rel {
                    cols: Vec::new(),
                    ..r.clone()
                };
                for v in vars {
                    out.set(v, r.col(v).cloned().unwrap_or_else(Col::null));
                }
                if *distinct {
                    Ok(self.derive(
                        out,
                        Select {
                            distinct: true,
                            ..Select::default()
                        },
                        Vec::new(),
                    ))
                } else {
                    Ok(out)
                }
            }
            Node::OrderLimit(input, keys, skip, limit) => {
                let r = self.compile(input)?;
                let order_by = self.sort_keys(keys, &r)?;
                let limit = limit.map(|n| self.params.push(SqlValue::Integer(n as i64)));
                let offset = skip.map(|n| self.params.push(SqlValue::Integer(n as i64)));
                Ok(self.derive(
                    r,
                    Select {
                        order_by,
                        limit,
                        offset,
                        ..Select::default()
                    },
                    Vec::new(),
                ))
            }
            Node::RowNumber(input, partition, keys, var) => {
                let r = self.compile(input)?;
                let order = self.sort_keys(keys, &r)?;
                let part: Vec<String> = partition
                    .iter()
                    .map(|v| r.col(v).map_or("NULL".to_string(), |c| c.sql.clone()))
                    .collect();
                let mut over = Vec::new();
                if !part.is_empty() {
                    over.push(format!("PARTITION BY {}", part.join(", ")));
                }
                if !order.is_empty() {
                    over.push(format!("ORDER BY {}", order.join(", ")));
                }
                let col = Col {
                    sql: format!("ROW_NUMBER() OVER ({})", over.join(" ")),
                    dom: Dom::Computed(VClass::Int),
                    mm: false,
                    eid_of: None,
                };
                Ok(self.derive(r, Select::default(), vec![(var.clone(), col)]))
            }
            Node::PadMissing(input, vars) => {
                let mut r = self.compile(input)?;
                for v in vars {
                    if r.col(v).is_none() {
                        r.set(v, Col::null());
                    }
                }
                Ok(r)
            }
            Node::Triple(_) | Node::Virtual(_) | Node::Volatile(_) | Node::Path(_) => {
                self.compile(node)
            }
        }
    }

    /// Compiles the root into one SELECT whose columns are `columns`, hoisting an
    /// OrderLimit at the root or under a root Project / Extend. Returns the SQL
    /// and the domain of each output column.
    pub fn root(&mut self, node: &Node, columns: &[Var]) -> Result<(String, Vec<Dom>)> {
        let mut cur = node;
        let mut proj: Option<(&Vec<Var>, bool)> = None;
        if let Node::Project(i, vars, distinct) = cur {
            proj = Some((vars, *distinct));
            cur = i;
        }
        let mut exts = Vec::new();
        while let Node::Extend(i, v, e) = cur {
            exts.push((v, e));
            cur = i;
        }
        let mut ol = None;
        if let Node::OrderLimit(i, keys, skip, limit) = cur {
            ol = Some((keys, *skip, *limit));
            cur = i;
        }
        let distinct = proj.is_some_and(|p| p.1);
        let mut r = self.compile(cur)?;
        let mut order_by = Vec::new();
        let (mut limit, mut offset) = (None, None);
        if let Some((keys, skip, lim)) = ol {
            if distinct && (skip.is_some() || lim.is_some()) {
                // limit before distinct: order and slice inside, keep the keys
                let exprs = self.sort_key_exprs(keys, &r)?;
                let extra: Vec<(Var, Col)> = exprs
                    .iter()
                    .enumerate()
                    .map(|(i, (e, _))| {
                        (
                            Var::new(format!("__k{i}")),
                            Col {
                                sql: e.clone(),
                                dom: Dom::Computed(VClass::Dynamic),
                                mm: true,
                                eid_of: None,
                            },
                        )
                    })
                    .collect();
                let inner_order = self.order_terms(&exprs);
                let l = lim.map(|n| self.params.push(SqlValue::Integer(n as i64)));
                let o = skip.map(|n| self.params.push(SqlValue::Integer(n as i64)));
                r = self.derive(
                    r,
                    Select {
                        order_by: inner_order,
                        limit: l,
                        offset: o,
                        ..Select::default()
                    },
                    extra,
                );
                let outer: Vec<(String, bool)> = exprs
                    .iter()
                    .enumerate()
                    .map(|(i, (_, d))| {
                        let c = r
                            .col(&Var::new(format!("__k{i}")))
                            .expect("key column")
                            .sql
                            .clone();
                        (c, *d)
                    })
                    .collect();
                order_by = self.order_terms(&outer);
            } else {
                order_by = self.sort_keys(keys, &r)?;
                limit = lim.map(|n| self.params.push(SqlValue::Integer(n as i64)));
                offset = skip.map(|n| self.params.push(SqlValue::Integer(n as i64)));
            }
        }
        for (v, e) in exts.iter().rev() {
            let val = self.value(e, &r)?;
            let c = self.val_col(val);
            r.set(v, c);
        }
        let mut list = Vec::new();
        let mut doms = Vec::new();
        for (i, v) in columns.iter().enumerate() {
            let c = r.col(v).cloned().unwrap_or_else(Col::null);
            list.push((c.sql.clone(), format!("c{i}")));
            doms.push(c.dom.clone());
        }
        if ol.is_some() {
            // tie-breakers: the raw output values
            for (e, _) in &list {
                if e != "NULL" {
                    order_by.push(e.clone());
                }
            }
        }
        let sql = self.select(
            &r,
            &Select {
                list,
                distinct,
                order_by,
                limit,
                offset,
                ..Select::default()
            },
        );
        if !self.regions.iter().any(|r| r.kind == RegionKind::Sql) {
            self.regions.insert(
                0,
                RegionInfo {
                    kind: RegionKind::Sql,
                    note: RouteNote::None,
                    aliases: Vec::new(),
                    query_plan: Vec::new(),
                },
            );
        }
        Ok((sql, doms))
    }

    /// Fails with `Unsupported` for a feature this change does not generate.
    pub fn unsupported<T>(&self, what: &str) -> Result<T> {
        Err(unsupported(what.to_string()))
    }
}
