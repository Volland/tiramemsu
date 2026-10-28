//! `CALL tiramemsu.datalog.query(program, goal) YIELD …`: a Datalog goal from
//! Cypher (`lat.md/query#Datalog Calls`). The procedure builds the IR leaf
//! `DatalogPattern` and runs it, so it returns the rows of the SPARQL service and
//! of `View::datalog`. Its arguments are constants or parameters, and a call runs
//! once per query execution: the relation is cached per program, goal and view,
//! so a later row, `CALL { }` subquery or `OPTIONAL MATCH` never runs it again.

use tm_core::Value;
use tm_ir::{DatalogPattern, DlProgramRef, Op, TermOrVar, Var};

use super::{core, Exec, Row};
use crate::ast::{Expr, ExprKind, Name};
use crate::error::{CResult, CypherError};
use crate::value::Val;

/// The procedure name (compared lower-case).
pub const NAME: &str = "tiramemsu.datalog.query";

fn shape(msg: impl std::fmt::Display) -> CypherError {
    core(tm_core::Error::DatalogCallShape {
        message: msg.to_string(),
    })
}

/// True for an expression that does not read the row: literals, parameters and
/// maps of those.
fn is_constant(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Lit(_) | ExprKind::Param(_) => true,
        ExprKind::Map(items) => items.iter().all(|(_, v)| is_constant(v)),
        _ => false,
    }
}

impl Exec<'_> {
    /// Runs the procedure for the input rows: `args` are the program (an IRI
    /// string or `{text: "…"}`) and the goal body.
    // @tg: implements:: [[openspec:datalog-query-calls#Cypher goal procedure]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:datalog-query-calls#Evaluated once per query]] {since: "2026-10-28"}
    pub(crate) fn exec_datalog_query(
        &mut self,
        rows: Vec<Row>,
        args: &[Expr],
        yields: &[(Name, Option<Name>)],
        standalone: bool,
    ) -> CResult<(Vec<Row>, Vec<String>)> {
        if args.len() != 2 {
            return Err(shape(format!(
                "{NAME} takes (program, goal), got {} arguments",
                args.len()
            )));
        }
        if let Some(a) = args.iter().find(|a| !is_constant(a)) {
            let _ = a;
            return Err(shape(
                "the program and the goal must be constants or parameters (a call runs once, \
                 never per row)",
            ));
        }
        let none = Row::new();
        let program = match self.eval(&args[0], &none)? {
            Val::Str(s) => DlProgramRef::Iri(TermOrVar::Const(Value::Iri(
                self.vocab.resolve_text(&s, true).map_err(shape)?,
            ))),
            Val::Map(m) => match (m.len(), m.get("text")) {
                (1, Some(Val::Str(t))) => DlProgramRef::Text(t.clone()),
                _ => return Err(shape("a program map is {text: \"…\"}")),
            },
            other => {
                return Err(shape(format!(
                    "the program is an IRI string or {{text: \"…\"}}, not a {}",
                    other.type_name()
                )))
            }
        };
        let goal = match self.eval(&args[1], &none)? {
            Val::Str(s) => s,
            other => {
                return Err(shape(format!(
                    "the goal is a string, not a {}",
                    other.type_name()
                )))
            }
        };
        let vars = tm_ir::goal_variables(&goal);
        let mut columns = vars.clone();
        columns.extend(["validFrom".to_string(), "validTo".to_string()]);
        let outs: Vec<(usize, String)> = if yields.is_empty() {
            if standalone {
                columns.iter().cloned().enumerate().collect()
            } else {
                Vec::new()
            }
        } else {
            let mut v = Vec::new();
            for (n, a) in yields {
                let Some(i) = columns.iter().position(|c| *c == n.text) else {
                    return Err(CypherError::eval(format!(
                        "{NAME}: no yield column `{}` (columns: {})",
                        n.text,
                        columns.join(", ")
                    )));
                };
                v.push((i, a.as_ref().unwrap_or(n).text.clone()));
            }
            v
        };
        let view = self.view();
        let key = format!("{program:?}\u{1}{goal}\u{1}{view:?}");
        let table = match self.datalog.get(&key) {
            Some(t) => t.clone(),
            None => {
                let mut call = DatalogPattern::new(program, goal, view);
                for g in &vars {
                    call.outputs.push((g.clone(), Var::new(format!("dl_{g}"))));
                }
                call.valid = Some((Var::new("dl_from"), Var::new("dl_to")));
                let res = self.run_op(Op::Datalog(call))?;
                let mut table = Vec::new();
                for r in &res.rows {
                    let mut cells = Vec::new();
                    for g in &vars {
                        let c = res.col(&format!("dl_{g}")).and_then(|i| r[i].clone());
                        cells.push(c.map_or(Val::Null, |v| Val::from_entity(&v)));
                    }
                    for name in ["dl_from", "dl_to"] {
                        let c = res.col(name).and_then(|i| r[i].clone());
                        cells.push(c.map_or(Val::Null, |v| Val::from_literal(&v)));
                    }
                    table.push(cells);
                }
                self.datalog.insert(key, table.clone());
                table
            }
        };
        let mut out = Vec::new();
        for r in rows {
            for cells in &table {
                let mut nr = r.clone();
                for (i, name) in &outs {
                    nr.insert(name.clone(), cells[*i].clone());
                }
                out.push(nr);
            }
        }
        Ok((out, outs.into_iter().map(|(_, n)| n).collect()))
    }
}
