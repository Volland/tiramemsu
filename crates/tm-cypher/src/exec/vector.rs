//! `CALL tiramemsu.vector.search(model, vector [, options]) YIELD …`: vector
//! recall from Cypher (`lat.md/query#Vector Recall`). The procedure lowers to an
//! IR vector recall joined with the hit's statement, so it runs the same
//! `tm_vector` recall as SPARQL's `tm:vectorMatch` and the Rust
//! `View::vector_search`.

use tm_core::Value;
use tm_ir::{GraphSel, Op, TermOrVar, TriplePattern, VectorPattern};

use super::{Exec, Row};
use crate::ast::{Expr, Name};
use crate::error::{CResult, CypherError};
use crate::value::Val;

/// The procedure name (compared lower-case).
pub const NAME: &str = "tiramemsu.vector.search";

/// The yield columns, in order.
pub const COLUMNS: [&str; 7] = [
    "statement",
    "subject",
    "predicate",
    "object",
    "similarity",
    "rank",
    "confidence",
];

fn err(msg: impl Into<String>) -> CypherError {
    CypherError::eval(format!("{NAME}: {}", msg.into()))
}

fn number(v: &Val) -> Option<f64> {
    match v {
        Val::Int(i) => Some(*i as f64),
        Val::Float(x) => Some(*x),
        _ => None,
    }
}

impl Exec<'_> {
    /// Runs the procedure once per input row: `args` are the model (an IRI or a
    /// name resolved like a graph name), the query vector (a list of numbers) and
    /// an optional map `{limit, graphs, predicates}`.
    // @lat: [[query#Vector Recall#Query Languages]]
    pub(crate) fn exec_vector_search(
        &mut self,
        rows: Vec<Row>,
        args: &[Expr],
        yields: &[(Name, Option<Name>)],
        standalone: bool,
    ) -> CResult<(Vec<Row>, Vec<String>)> {
        if args.len() < 2 || args.len() > 3 {
            return Err(err(format!("takes 2 or 3 arguments, got {}", args.len())));
        }
        let outs: Vec<(usize, String)> = if yields.is_empty() {
            if standalone {
                COLUMNS
                    .iter()
                    .enumerate()
                    .map(|(i, c)| (i, c.to_string()))
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            let mut v = Vec::new();
            for (n, a) in yields {
                let Some(i) = COLUMNS.iter().position(|c| *c == n.text) else {
                    return Err(err(format!("no yield column `{}`", n.text)));
                };
                v.push((i, a.as_ref().unwrap_or(n).text.clone()));
            }
            v
        };
        let mut out = Vec::new();
        for r in rows {
            let model = match self.eval(&args[0], &r)? {
                Val::Str(s) => self.vocab.resolve_text(&s, true).map_err(err)?,
                Val::Null => continue,
                other => return Err(err(format!("the model is a {}", other.type_name()))),
            };
            let vector = match self.eval(&args[1], &r)? {
                Val::List(xs) => {
                    let mut parts = Vec::with_capacity(xs.len());
                    for x in &xs {
                        let n = number(x).ok_or_else(|| err("the vector is a list of numbers"))?;
                        if !n.is_finite() {
                            return Err(err("the vector has a NaN or infinite component"));
                        }
                        parts.push(format!("{n}"));
                    }
                    format!("[{}]", parts.join(","))
                }
                Val::Null => continue,
                other => return Err(err(format!("the vector is a {}", other.type_name()))),
            };
            let opts = match args.get(2).map(|a| self.eval(a, &r)).transpose()? {
                None | Some(Val::Null) => Default::default(),
                Some(Val::Map(m)) => m,
                Some(other) => return Err(err(format!("options are a {}", other.type_name()))),
            };
            let mut t = VectorPattern::new(
                TermOrVar::Const(Value::Iri(model)),
                TermOrVar::Const(Value::Str(vector)),
                "e",
                self.view(),
            );
            t.similarity = Some("similarity".into());
            t.rank = Some("rank".into());
            t.confidence = Some("confidence".into());
            for (k, v) in &opts {
                match (k.as_str(), v) {
                    ("limit", Val::Int(n)) if *n >= 0 => {
                        t.limit = Some(u32::try_from(*n).unwrap_or(u32::MAX))
                    }
                    ("graphs", Val::List(gs)) => {
                        let mut set = Vec::new();
                        for g in gs {
                            let Val::Str(g) = g else {
                                return Err(err("graphs is a list of graph names"));
                            };
                            let iri = self.vocab.resolve_text(g, true).map_err(err)?;
                            set.push(TermOrVar::Const(Value::Iri(iri)));
                        }
                        if set.is_empty() {
                            continue;
                        }
                        t.graph = GraphSel::Set(set);
                    }
                    ("predicates", Val::List(ps)) => {
                        for p in ps {
                            let Val::Str(p) = p else {
                                return Err(err("predicates is a list of predicate names"));
                            };
                            let iri = self.vocab.resolve_text(p, true).map_err(err)?;
                            t.predicates.push(TermOrVar::Const(Value::Iri(iri)));
                        }
                    }
                    (k, _) => return Err(err(format!("unknown or invalid option `{k}`"))),
                }
            }
            let empty = |k: &str| matches!(opts.get(k), Some(Val::List(xs)) if xs.is_empty());
            if empty("graphs") || empty("predicates") {
                // an empty graph or predicate list selects nothing
                continue;
            }
            let mut stmt = TriplePattern::new("?s", "?p", "?o", self.view());
            stmt.eid = Some("e".into());
            let op = Op::join(vec![Op::Vector(t), Op::Triple(stmt)]).order_limit(
                vec![tm_ir::Key::asc(tm_ir::Expr::var("rank"))],
                None,
                None,
            );
            let res = self.run_op(op)?;
            let col = |name: &str| res.col(name);
            for row in &res.rows {
                let get = |name: &str| col(name).and_then(|i| row[i].clone());
                let float = |v: Option<Value>| match v {
                    Some(Value::Double(x)) => Val::Float(x),
                    Some(Value::Int(i)) => Val::Float(i as f64),
                    _ => Val::Null,
                };
                let cells: [Val; 7] = [
                    get("e").map_or(Val::Null, Val::Node),
                    get("s").map_or(Val::Null, Val::Node),
                    match get("p") {
                        Some(Value::Iri(p)) => Val::Str(self.vocab.render(&p)),
                        _ => Val::Null,
                    },
                    get("o").map_or(Val::Null, |o| Val::from_entity(&o)),
                    float(get("similarity")),
                    match get("rank") {
                        Some(Value::Int(i)) => Val::Int(i),
                        _ => Val::Null,
                    },
                    float(get("confidence")),
                ];
                let mut nr = r.clone();
                for (i, name) in &outs {
                    nr.insert(name.clone(), cells[*i].clone());
                }
                out.push(nr);
            }
        }
        let cols = outs.into_iter().map(|(_, n)| n).collect();
        Ok((out, cols))
    }
}
