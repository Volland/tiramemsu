//! Running a prepared query on one connection (design D3, D14): plan lookups, the
//! single SQL statement and decoding all use the same executor, so they share
//! one snapshot. The caller opens the read transaction on a reader (or runs on
//! the writer inside a speculation's savepoint).

use std::collections::HashMap;

use tm_core::{budget, Executor, Result, SqlValue, Value};
use tm_ir::{IrQuery, Var};

use crate::decode::{CacheMode, Decoder};
use crate::plan::analyze::Dom;
use crate::plan::normalize::Planner;
use crate::plan::{Cell, Node, PValues};
use crate::result::{Explain, QueryResult, RegionInfo, RegionKind, ResultValue};
use crate::sqlgen::Gen;
use crate::QueryEngine;

/// A validated query with its parameters bound, ready to run on any connection.
///
/// Made by [`QueryEngine::prepare`]. It holds no connection and no snapshot, so
/// it can be run more than once; each run re-plans against the data it sees.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub(crate) query: IrQuery,
    pub(crate) columns: Vec<Var>,
}

impl Prepared {
    /// The result columns.
    pub fn columns(&self) -> &[Var] {
        &self.columns
    }

    /// The bound query.
    pub fn query(&self) -> &IrQuery {
        &self.query
    }
}

/// Where a query runs: an executor handle plus the cache mode.
pub struct ExecContext<'a> {
    /// The connection.
    pub exec: &'a mut dyn Executor,
    /// Shared (reader) or scoped (speculation) term caching.
    pub cache: CacheMode,
}

enum Body {
    Empty,
    Rows(PValues, Vec<Var>),
    Sql {
        sql: String,
        params: Vec<SqlValue>,
        doms: Vec<Dom>,
    },
}

struct Planned {
    body: Body,
    regions: Vec<RegionInfo>,
    synthetic: HashMap<i64, Value>,
}

fn constant_rows(node: &Node) -> Option<(PValues, Option<Vec<Var>>)> {
    match node {
        Node::Values(v) => Some((v.clone(), None)),
        Node::Project(i, vars, false) => match &**i {
            Node::Values(v) => Some((v.clone(), Some(vars.clone()))),
            _ => None,
        },
        _ => None,
    }
}

fn plan(engine: &QueryEngine, exec: &mut dyn Executor, p: &Prepared) -> Result<Planned> {
    let mut planner = Planner::new(exec, &p.query);
    let node = planner.plan(&p.query.root)?;
    let synthetic = std::mem::take(&mut planner.synthetic);
    if let Node::Empty(_) = node {
        return Ok(Planned {
            body: Body::Empty,
            regions: Vec::new(),
            synthetic,
        });
    }
    if let Some((values, proj)) = constant_rows(&node) {
        return Ok(Planned {
            body: Body::Rows(values, proj.unwrap_or_else(|| p.columns.clone())),
            regions: Vec::new(),
            synthetic,
        });
    }
    let mut gen = Gen::new(p.query.semantics, &engine.registry, &engine.options);
    gen.set_synthetic(&synthetic);
    let (sql, doms) = gen.root(&node, &p.columns)?;
    Ok(Planned {
        body: Body::Sql {
            sql,
            params: gen.params.values().to_vec(),
            doms,
        },
        regions: gen.regions,
        synthetic,
    })
}

/// Plans, runs and decodes a prepared query.
pub fn run(engine: &QueryEngine, ctx: ExecContext<'_>, p: &Prepared) -> Result<QueryResult> {
    let exec = ctx.exec;
    let planned = plan(engine, &mut *exec, p)?;
    let mut dec = Decoder::new(engine.term_cache(), ctx.cache, &planned.synthetic);
    let mut rows = Vec::new();
    // only an operation with a budget pays for counting decoded bytes
    let metered = budget::active();
    match &planned.body {
        Body::Empty => {}
        Body::Rows(v, proj) => {
            for r in &v.rows {
                let mut out = Vec::new();
                for col in &p.columns {
                    let cell = match (v.vars.iter().position(|x| x == col), proj.contains(col)) {
                        (Some(i), true) => match &r[i] {
                            None => None,
                            Some(Cell::Id(id)) => {
                                Some(ResultValue::Term(dec.term(&mut *exec, *id)?))
                            }
                            Some(Cell::Int(n)) => Some(ResultValue::Term(Value::Int(*n))),
                            Some(Cell::EmptyList) => Some(ResultValue::List(Vec::new())),
                        },
                        _ => None,
                    };
                    out.push(cell);
                }
                if metered {
                    budget::charge_rows(1)?;
                    charge_row(&out)?;
                }
                rows.push(out);
            }
        }
        Body::Sql { sql, params, doms } => {
            engine.run_hook();
            // an operation budget counts rows as SQL produces them, so a runaway
            // result fails before it is materialised (lat.md/query#Query Budgets)
            let mut raw = Vec::new();
            exec.query(sql, params, &mut |r| {
                if metered {
                    budget::charge_rows(1)?;
                }
                raw.push(r.to_vec());
                Ok(())
            })?;
            dec.stats.sql_executed = true;
            for r in raw {
                let mut out = Vec::with_capacity(doms.len());
                for (v, d) in r.iter().zip(doms) {
                    out.push(dec.cell(&mut *exec, v, d)?);
                }
                if metered {
                    charge_row(&out)?;
                }
                rows.push(out);
            }
        }
    }
    Ok(QueryResult {
        columns: p.columns.clone(),
        rows,
        stats: dec.stats,
    })
}

/// Charges the decoded bytes of one result row to the operation budget.
fn charge_row(row: &[Option<ResultValue>]) -> Result<()> {
    fn bytes(c: &Option<ResultValue>) -> u64 {
        match c {
            None => 8,
            Some(ResultValue::Term(v)) => budget::value_bytes(v),
            Some(ResultValue::List(xs)) => 8 + xs.iter().map(bytes).sum::<u64>(),
        }
    }
    budget::charge_bytes(row.iter().map(bytes).sum())
}

fn scan_target(detail: &str) -> Option<&str> {
    let mut words = detail.split_whitespace();
    match words.next()? {
        "SCAN" | "SEARCH" => words.next(),
        _ => None,
    }
}

/// Explains a prepared query: routing, SQL, parameters and `EXPLAIN QUERY PLAN`
/// with the real parameters bound. The query itself is never stepped.
pub fn explain(engine: &QueryEngine, exec: &mut dyn Executor, p: &Prepared) -> Result<Explain> {
    let planned = plan(engine, &mut *exec, p)?;
    let Body::Sql { sql, params, .. } = planned.body else {
        return Ok(Explain {
            regions: planned.regions,
            short_circuit: true,
            sql: None,
            params: Vec::new(),
            query_plan: Vec::new(),
        });
    };
    let rows = exec.rows(&format!("EXPLAIN QUERY PLAN {sql}"), &params)?;
    let plan: Vec<String> = rows
        .iter()
        .filter_map(|r| r.last().and_then(|d| d.as_str()).map(str::to_string))
        .collect();
    let mut regions = planned.regions;
    for reg in &mut regions {
        reg.query_plan = plan
            .iter()
            .filter(|d| scan_target(d).is_some_and(|t| reg.aliases.iter().any(|a| a == t)))
            .cloned()
            .collect();
        if reg.kind == RegionKind::Sql && reg.aliases.is_empty() {
            reg.query_plan = plan.clone();
        }
    }
    Ok(Explain {
        regions,
        short_circuit: false,
        sql: Some(sql),
        params,
        query_plan: plan,
    })
}
