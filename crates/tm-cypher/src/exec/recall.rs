//! `CALL tiramemsu.recall({text, model, vector, mode, limit, expand, graphs}) YIELD …`:
//! hybrid recall from Cypher (`lat.md/query#Hybrid Recall`). The procedure lowers
//! to an IR recall joined with each result's statement, so it runs the same
//! `tm_recall` as SPARQL's `tm:recall*` patterns and the Rust `View::recall`.

use tm_core::{TextMode, Value};
use tm_ir::{GraphSel, Op, RecallPattern, TermOrVar, TriplePattern};

use super::{Exec, Row};
use crate::ast::{Expr, Name};
use crate::error::{CResult, CypherError};
use crate::value::Val;

/// The procedure name (compared lower-case).
pub const NAME: &str = "tiramemsu.recall";

/// The yield columns, in order.
pub const COLUMNS: [&str; 7] = [
    "statement",
    "subject",
    "predicate",
    "object",
    "score",
    "rank",
    "via",
];

fn err(msg: impl Into<String>) -> CypherError {
    CypherError::eval(format!("{NAME}: {}", msg.into()))
}

impl Exec<'_> {
    /// Runs the procedure once per input row: `args` is one map with `text` and/or
    /// `model` with `vector`, and optionally `mode`, `limit`, `expand` and `graphs`.
    // @lat: [[query#Hybrid Recall#Query Languages]]
    pub(crate) fn exec_recall(
        &mut self,
        rows: Vec<Row>,
        args: &[Expr],
        yields: &[(Name, Option<Name>)],
        standalone: bool,
    ) -> CResult<(Vec<Row>, Vec<String>)> {
        if args.len() != 1 {
            return Err(err(format!("takes 1 argument (a map), got {}", args.len())));
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
            let opts = match self.eval(&args[0], &r)? {
                Val::Map(m) => m,
                Val::Null => continue,
                other => return Err(err(format!("the argument is a {}", other.type_name()))),
            };
            let mut t = RecallPattern::new("e", self.view());
            t.score = Some("score".into());
            t.rank = Some("rank".into());
            t.via = Some("via".into());
            let mut skip = false;
            for (k, v) in &opts {
                match (k.as_str(), v) {
                    ("text", Val::Str(s)) => t.text = Some(TermOrVar::Const(Value::Str(s.clone()))),
                    ("model", Val::Str(s)) => {
                        let iri = self.vocab.resolve_text(s, true).map_err(err)?;
                        t.model = Some(TermOrVar::Const(Value::Iri(iri)));
                    }
                    ("vector", Val::List(xs)) => {
                        let mut parts = Vec::with_capacity(xs.len());
                        for x in xs {
                            let n = match x {
                                Val::Int(i) => *i as f64,
                                Val::Float(f) if f.is_finite() => *f,
                                _ => return Err(err("vector is a list of finite numbers")),
                            };
                            parts.push(format!("{n}"));
                        }
                        t.vector = Some(TermOrVar::Const(Value::Str(format!(
                            "[{}]",
                            parts.join(",")
                        ))));
                    }
                    ("mode", Val::Str(m)) => {
                        t.mode = TextMode::from_name(m)
                            .ok_or_else(|| err("mode is 'all', 'any' or 'phrase'"))?
                    }
                    ("limit", Val::Int(n)) if *n >= 0 => {
                        t.limit = Some(u32::try_from(*n).unwrap_or(u32::MAX))
                    }
                    ("expand", Val::Int(n)) if *n >= 0 => {
                        t.expand = u32::try_from(*n).unwrap_or(u32::MAX)
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
                            // an empty graph list selects nothing
                            skip = true;
                        } else {
                            t.graph = GraphSel::Set(set);
                        }
                    }
                    (k, _) => return Err(err(format!("unknown or invalid option `{k}`"))),
                }
            }
            if t.text.is_none() && t.vector.is_none() {
                return Err(err("give `text`, or `model` with `vector`, or both"));
            }
            if t.vector.is_some() != t.model.is_some() {
                return Err(err("the vector channel needs both `model` and `vector`"));
            }
            if skip {
                continue;
            }
            let mut stmt = TriplePattern::new("?s", "?p", "?o", self.view());
            stmt.eid = Some("e".into());
            let op = Op::join(vec![Op::Recall(t), Op::Triple(stmt)]).order_limit(
                vec![tm_ir::Key::asc(tm_ir::Expr::var("rank"))],
                None,
                None,
            );
            let res = self.run_op(op)?;
            let col = |name: &str| res.col(name);
            for row in &res.rows {
                let get = |name: &str| col(name).and_then(|i| row[i].clone());
                let cells: [Val; 7] = [
                    get("e").map_or(Val::Null, Val::Node),
                    get("s").map_or(Val::Null, Val::Node),
                    match get("p") {
                        Some(Value::Iri(p)) => Val::Str(self.vocab.render(&p)),
                        _ => Val::Null,
                    },
                    get("o").map_or(Val::Null, |o| Val::from_entity(&o)),
                    match get("score") {
                        Some(Value::Double(x)) => Val::Float(x),
                        Some(Value::Int(i)) => Val::Float(i as f64),
                        _ => Val::Null,
                    },
                    match get("rank") {
                        Some(Value::Int(i)) => Val::Int(i),
                        _ => Val::Null,
                    },
                    get("via").map_or(Val::Null, Val::Node),
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
