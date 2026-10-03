//! The native cyclic-join operator (OpenSpec change `add-lftj-operator`): a
//! leapfrog triejoin for pure cyclic basic graph patterns, exposed to SQL as the
//! table-valued function `tm_lftj(spec)` with output columns `c0 … c31`.
//!
//! The planner routes a region here only when LFTJ is enabled, the operator is
//! registered, the region is a pure cyclic triple-pattern join and the estimate
//! policy agrees ([`crate::plan::route`]). The operator reads through the calling
//! statement's connection, so it sees that statement's snapshot (or the
//! speculative state inside `with`):
//!
//! 1. each pattern becomes one sorted access path: an ordered scan of `triple`
//!    with the pattern's constants, its own view predicates and, under set
//!    semantics, the canonical-eid predicate, all from the same code that
//!    generates SQL patterns, so both routes select the same statements;
//! 2. [`join::leapfrog`] intersects the access paths one variable at a time;
//! 3. every binding passes the relationship-isomorphism test of its group and
//!    becomes one output row; every pattern has an eid variable (a hidden one if
//!    the query binds none), so parallel statements keep their multiplicity.
//!
//! The operation budget is polled throughout; a cancelled or timed-out call fails
//! the statement with the typed error and returns no rows.

pub mod join;
pub mod spec;

use std::collections::BTreeMap;

use tm_core::{
    budget, ConnTableFunction, Error, Executor, HostRegistry, Result, SqlError, SqlValue,
};
use tm_ir::{Semantics, Var};

use crate::native::{NativeKind, NativeOperator, OperatorRegistry, PlannerOptions};
use crate::plan::{PTerm, PTriple};
use crate::sqlgen::Gen;
pub use join::Relation;
pub use spec::{LftjPattern, LftjSpec, Slot};

/// The SQL name.
pub const NAME: &str = "tm_lftj";

/// The number of output columns of `tm_lftj`: a region with more variables to
/// return stays in SQL.
pub const MAX_COLUMNS: usize = 32;

fn arg_err(msg: impl std::fmt::Display) -> Error {
    Error::Sqlite(SqlError::new(
        SqlError::ERROR,
        format!("{NAME}: spec: {msg}"),
    ))
}

/// The native cyclic-join operator: registers `tm_lftj` on every connection.
///
/// The `tiramemsu` facade installs it when `OpenOptions::planner.lftj.enabled` is
/// set; without it an enabled planner still routes cyclic patterns to SQL
/// ([`RouteNote::LftjUnavailable`](crate::RouteNote::LftjUnavailable)).
///
/// # Example
///
/// ```
/// use std::sync::Arc;
/// use tm_exec::{LftjOperator, NativeKind, NativeOperator, OperatorRegistry};
///
/// let mut reg = OperatorRegistry::new();
/// reg.add(Arc::new(LftjOperator));
/// assert_eq!(reg.lftj().map(|o| o.kind()), Some(NativeKind::Lftj));
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct LftjOperator;

impl NativeOperator for LftjOperator {
    fn kind(&self) -> NativeKind {
        NativeKind::Lftj
    }

    fn tvf_name(&self) -> &'static str {
        NAME
    }

    fn register(&self, host: &mut dyn HostRegistry) -> Result<()> {
        host.register_conn_table(ConnTableFunction {
            name: NAME.to_string(),
            args: vec!["spec".to_string()],
            columns: (0..MAX_COLUMNS).map(|i| format!("c{i}")).collect(),
            pushdown: Vec::new(),
            func: std::sync::Arc::new(call),
        })
    }
}

/// Evaluates one call of `tm_lftj(spec)`.
// @lat: [[query#Physical Planning#LFTJ]]
pub fn call(exec: &mut dyn Executor, args: &[SqlValue]) -> Result<Vec<Vec<SqlValue>>> {
    let spec = match args.first() {
        Some(SqlValue::Text(t)) => LftjSpec::parse(t).map_err(arg_err)?,
        _ => return Err(arg_err("expected plan text")),
    };
    let mut rows = Vec::new();
    run(exec, &spec, &mut |r| {
        rows.push(r.iter().map(|v| SqlValue::Integer(*v)).collect());
        Ok(())
    })?;
    Ok(rows)
}

/// Runs a region plan on `exec`, calling `out` with the output values of every
/// result row (bag semantics: one call per solution).
pub fn run(
    exec: &mut dyn Executor,
    spec: &LftjSpec,
    out: &mut dyn FnMut(&[i64]) -> Result<()>,
) -> Result<()> {
    budget::check()?;
    let mut rels = Vec::with_capacity(spec.patterns.len());
    for p in &spec.patterns {
        let rel = access_path(exec, p)?;
        if rel.data.is_empty() {
            return Ok(());
        }
        rels.push(rel);
    }
    // eid pairs that must differ (relationship isomorphism, as the SQL route's
    // `tI.eid <> tJ.eid`)
    let mut distinct = Vec::new();
    for (i, a) in spec.patterns.iter().enumerate() {
        for b in &spec.patterns[i + 1..] {
            let same_group = a.iso_group.is_some() && a.iso_group == b.iso_group;
            let preds_differ = matches!((a.pred(), b.pred()), (Some(x), Some(y)) if x != y);
            if same_group && !preds_differ {
                distinct.push((a.eid, b.eid));
            }
        }
    }
    let mut row = vec![0i64; spec.out.len()];
    join::leapfrog(spec.vars, &rels, &mut |b| {
        if distinct.iter().any(|&(x, y)| b[x] == b[y]) {
            return Ok(());
        }
        for (slot, v) in row.iter_mut().zip(&spec.out) {
            *slot = b[*v];
        }
        out(&row)
    })
}

fn var_name(v: usize) -> Var {
    Var::new(format!("v{v}"))
}

fn pterm(s: &Slot) -> PTerm {
    match s {
        Slot::Var(v) => PTerm::Var(var_name(*v)),
        Slot::Id(id) => PTerm::Id(*id),
    }
}

/// The SQL of one pattern's scan: the pattern compiled exactly as the SQL route
/// compiles it, selecting its variables in trie order.
fn scan_sql(p: &LftjPattern, ordered: bool) -> Result<(String, Vec<SqlValue>, Vec<usize>)> {
    let reg = OperatorRegistry::new();
    let opts = PlannerOptions::default();
    let mut gen = Gen::new(Semantics::sparql(), &reg, &opts);
    let t = PTriple {
        s: pterm(&p.s),
        p: pterm(&p.p),
        o: pterm(&p.o),
        eid: Some(var_name(p.eid)),
        view: p.view,
        iso_group: None,
        canonical: p.canonical,
    };
    let rel = gen.triple(&t)?;
    let cols = p.var_list();
    let mut list = Vec::with_capacity(cols.len());
    for v in &cols {
        let c = rel
            .col(&var_name(*v))
            .ok_or_else(|| arg_err(format!("variable {v} has no column")))?;
        list.push(c.sql.clone());
    }
    let mut sql = format!("SELECT {} FROM {}", list.join(", "), rel.items[0].sql);
    if !rel.conds.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&rel.conds.join(" AND "));
    }
    if ordered {
        sql.push_str(" ORDER BY ");
        sql.push_str(&list.join(", "));
    }
    Ok((sql, gen.params.values().to_vec(), cols))
}

/// The sorted access path of one pattern, read in the caller's snapshot.
fn access_path(exec: &mut dyn Executor, p: &LftjPattern) -> Result<Relation> {
    let (sql, params, cols) = scan_sql(p, true)?;
    let mut data = Vec::new();
    exec.query(&sql, &params, &mut |r| {
        budget::poll()?;
        for v in r {
            data.push(v.as_i64().unwrap_or_default());
        }
        Ok(())
    })?;
    Ok(Relation { cols, data })
}

/// The estimate of the routing policy: the largest number of statements any one
/// pattern matches in its view, counting at most `cap` per pattern.
pub fn estimate(exec: &mut dyn Executor, patterns: &[LftjPattern], cap: u64) -> Result<u64> {
    let mut best = 0u64;
    for p in patterns {
        // an upper bound: without the canonical-eid predicate
        let loose = LftjPattern {
            canonical: false,
            ..p.clone()
        };
        let (sql, mut params, _) = scan_sql(&loose, false)?;
        params.push(SqlValue::Integer(i64::try_from(cap).unwrap_or(i64::MAX)));
        let n = params.len();
        let count = exec
            .query_i64(&format!("SELECT count(*) FROM ({sql} LIMIT ?{n})"), &params)?
            .unwrap_or(0);
        best = best.max(u64::try_from(count).unwrap_or(0));
        if best >= cap {
            break;
        }
    }
    Ok(best)
}

/// Why a cyclic region cannot be planned natively.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Unplannable {
    /// More output variables than [`MAX_COLUMNS`].
    TooManyColumns,
}

/// A region plan plus the query variables of its output columns.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionPlan {
    /// The operator's plan.
    pub spec: LftjSpec,
    /// The query variable of each output column; `None` for a hidden eid exposed
    /// only for relationship isomorphism with patterns outside the region.
    pub columns: Vec<Option<Var>>,
    /// Per pattern, the output column of its eid when it is in an isomorphism
    /// group.
    pub iso_columns: Vec<Option<usize>>,
}

/// Plans a pure triple-pattern join: numbers its variables in join order
/// (subject/object/predicate variables first, most shared first and connected to
/// those already chosen, then eid variables, then hidden eids) and lists the
/// output columns. With `iso`, the eid of every pattern in a group is an output
/// column too.
pub fn plan_region(
    triples: &[&PTriple],
    iso: bool,
) -> std::result::Result<RegionPlan, Unplannable> {
    // query variables: (occurrences, first seen, eid-position) by name
    let mut info: BTreeMap<Var, (usize, usize, bool)> = BTreeMap::new();
    let mut seen = 0usize;
    let mut note = |v: &Var, eid: bool, info: &mut BTreeMap<Var, (usize, usize, bool)>| {
        let e = info.entry(v.clone()).or_insert_with(|| {
            seen += 1;
            (0, seen, false)
        });
        e.0 += 1;
        e.2 |= eid;
    };
    let mut pattern_vars: Vec<Vec<Var>> = Vec::new();
    for t in triples {
        let mut vs = Vec::new();
        for pos in [&t.s, &t.p, &t.o] {
            if let PTerm::Var(v) = pos {
                if !vs.contains(v) {
                    note(v, false, &mut info);
                    vs.push(v.clone());
                }
            }
        }
        if let Some(e) = &t.eid {
            if !vs.contains(e) {
                note(e, true, &mut info);
                vs.push(e.clone());
            }
        }
        pattern_vars.push(vs);
    }
    // greedy order over non-eid variables
    let mut order: Vec<Var> = Vec::new();
    let mut rest: Vec<Var> = info
        .iter()
        .filter(|(_, i)| !i.2)
        .map(|(v, _)| v.clone())
        .collect();
    while !rest.is_empty() {
        let connected = |v: &Var| {
            pattern_vars
                .iter()
                .any(|ps| ps.contains(v) && ps.iter().any(|x| order.contains(x)))
        };
        let best = rest
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| {
                let (n, first, _) = info[*v];
                (connected(v), n, std::cmp::Reverse(first))
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        order.push(rest.remove(best));
    }
    let mut eids: Vec<(&Var, &(usize, usize, bool))> = info.iter().filter(|(_, i)| i.2).collect();
    eids.sort_by_key(|(_, i)| i.1);
    order.extend(eids.into_iter().map(|(v, _)| v.clone()));
    let index = |v: &Var| order.iter().position(|x| x == v).expect("ordered variable");
    let mut vars = order.len();
    let mut columns: Vec<Option<Var>> = order.iter().cloned().map(Some).collect();
    let mut out: Vec<usize> = (0..order.len()).collect();
    let mut patterns = Vec::new();
    let mut iso_columns = Vec::new();
    for t in triples {
        let slot = |p: &PTerm| match p {
            PTerm::Var(v) => Slot::Var(index(v)),
            PTerm::Id(id) => Slot::Id(*id),
        };
        let eid = match &t.eid {
            Some(e) => index(e),
            None => {
                vars += 1;
                vars - 1
            }
        };
        let grouped = iso && t.iso_group.is_some();
        iso_columns.push(if grouped {
            match out.iter().position(|v| *v == eid) {
                Some(c) => Some(c),
                None => {
                    out.push(eid);
                    columns.push(None);
                    Some(out.len() - 1)
                }
            }
        } else {
            None
        });
        patterns.push(LftjPattern {
            s: slot(&t.s),
            p: slot(&t.p),
            o: slot(&t.o),
            eid,
            view: t.view,
            canonical: t.canonical,
            iso_group: if iso { t.iso_group } else { None },
        });
    }
    if out.len() > MAX_COLUMNS {
        return Err(Unplannable::TooManyColumns);
    }
    Ok(RegionPlan {
        spec: LftjSpec {
            vars,
            out,
            patterns,
        },
        columns,
        iso_columns,
    })
}
