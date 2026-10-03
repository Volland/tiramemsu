//! `CALL tiramemsu.text.search(query [, options]) YIELD …`: text recall from
//! Cypher (`lat.md/query#Text Recall`). The procedure lowers to an IR text
//! recall joined with the hit's statement, so it runs the same `tm_text` recall
//! as SPARQL's `tm:textMatch` and the Rust `View::text_search`.

use tm_core::{TextMode, Value};
use tm_ir::{GraphSel, Op, TermOrVar, TextPattern, TriplePattern};

use super::{Exec, Row};
use crate::ast::{Expr, Name};
use crate::error::{CResult, CypherError};
use crate::value::Val;

/// The procedure name (compared lower-case).
pub const NAME: &str = "tiramemsu.text.search";

/// The yield columns, in order.
pub const COLUMNS: [&str; 7] = [
    "statement",
    "subject",
    "predicate",
    "text",
    "score",
    "rank",
    "confidence",
];

fn err(msg: impl Into<String>) -> CypherError {
    CypherError::eval(format!("{NAME}: {}", msg.into()))
}

impl Exec<'_> {
    /// Runs the procedure once per input row: `args` are the query text and an
    /// optional map `{limit, mode, graphs}`.
    pub(crate) fn exec_text_search(
        &mut self,
        rows: Vec<Row>,
        args: &[Expr],
        yields: &[(Name, Option<Name>)],
        standalone: bool,
    ) -> CResult<(Vec<Row>, Vec<String>)> {
        if args.is_empty() || args.len() > 2 {
            return Err(err(format!("takes 1 or 2 arguments, got {}", args.len())));
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
            let query = match self.eval(&args[0], &r)? {
                Val::Str(s) => s,
                Val::Null => continue,
                other => return Err(err(format!("the query is a {}", other.type_name()))),
            };
            let opts = match args.get(1).map(|a| self.eval(a, &r)).transpose()? {
                None | Some(Val::Null) => Default::default(),
                Some(Val::Map(m)) => m,
                Some(other) => return Err(err(format!("options are a {}", other.type_name()))),
            };
            let mut t = TextPattern::new(TermOrVar::Const(Value::Str(query)), "e", self.view());
            t.score = Some("score".into());
            t.rank = Some("rank".into());
            t.confidence = Some("confidence".into());
            for (k, v) in &opts {
                match (k.as_str(), v) {
                    ("limit", Val::Int(n)) if *n >= 0 => {
                        t.limit = Some(u32::try_from(*n).unwrap_or(u32::MAX))
                    }
                    ("mode", Val::Str(m)) => {
                        t.mode = TextMode::from_name(m)
                            .ok_or_else(|| err("mode is 'all', 'any' or 'phrase'"))?
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
                    (k, _) => return Err(err(format!("unknown or invalid option `{k}`"))),
                }
            }
            if matches!(opts.get("graphs"), Some(Val::List(gs)) if gs.is_empty()) {
                // an empty graph list selects nothing
                continue;
            }
            let mut stmt = TriplePattern::new("?s", "?p", "?o", self.view());
            stmt.eid = Some("e".into());
            let op = Op::join(vec![Op::Text(t), Op::Triple(stmt)]).order_limit(
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
                    match get("o") {
                        Some(Value::Str(s)) => Val::Str(s),
                        Some(Value::LangStr { lex, .. }) => Val::Str(lex),
                        _ => Val::Null,
                    },
                    match get("score") {
                        Some(Value::Double(x)) => Val::Float(x),
                        Some(Value::Int(i)) => Val::Float(i as f64),
                        _ => Val::Null,
                    },
                    match get("rank") {
                        Some(Value::Int(i)) => Val::Int(i),
                        _ => Val::Null,
                    },
                    match get("confidence") {
                        Some(Value::Double(x)) => Val::Float(x),
                        Some(Value::Int(i)) => Val::Float(i as f64),
                        _ => Val::Null,
                    },
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
