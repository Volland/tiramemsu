//! `SERVICE <urn:tiramemsu:dl:query> { [] dl:goal "…" ; dl:program <iri> | dl:text "…" ;
//! dl:validFrom ?f ; dl:validTo ?t }` → [`Op::Datalog`] (`lat.md/query#Datalog Calls`).
//!
//! The front end has no Datalog parser: the goal's variables are read textually
//! ([`tm_ir::goal_variables`]) and each binds the SPARQL variable of the same name.

use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern};
use tm_core::vocab::{DL_GOAL, DL_PROGRAM, DL_TEXT, DL_VALID_FROM, DL_VALID_TO};
use tm_core::{Error, Result, Value};
use tm_ir::{DatalogPattern, DlProgramRef, Op, TermOrVar, View};

use super::Lowerer;
use crate::terms;

fn shape(msg: impl std::fmt::Display) -> Error {
    Error::DatalogCallShape {
        message: msg.to_string(),
    }
}

impl Lowerer<'_> {
    /// Lowers the group of a `SERVICE <urn:tiramemsu:dl:query>`.
    // @tg: implements:: [[openspec:datalog-query-calls#SPARQL goal service]] {since: "2026-10-28"}
    pub(super) fn datalog_service(&mut self, inner: &GraphPattern, view: View) -> Result<Op> {
        let GraphPattern::Bgp { patterns } = inner else {
            return Err(shape(
                "the service group holds only the call's property list on one subject",
            ));
        };
        let mut subject: Option<&TermPattern> = None;
        let (mut goal, mut program, mut text) = (None::<String>, None::<TermOrVar>, None::<String>);
        let (mut from, mut to) = (None, None);
        for t in patterns {
            match subject {
                None => subject = Some(&t.subject),
                Some(s) if s == &t.subject => {}
                Some(_) => return Err(shape("all properties of a call share one subject")),
            }
            if !matches!(
                t.subject,
                TermPattern::BlankNode(_) | TermPattern::Variable(_)
            ) {
                return Err(shape("the subject of a call is `[]` or a variable"));
            }
            let NamedNodePattern::NamedNode(p) = &t.predicate else {
                return Err(shape(
                    "a call's property is dl:goal, dl:program, dl:text, dl:validFrom or dl:validTo",
                ));
            };
            let dup = |name: &str| shape(format!("{name} given twice"));
            match p.as_str() {
                DL_GOAL => {
                    if goal.is_some() {
                        return Err(dup("dl:goal"));
                    }
                    goal = Some(string_const(&t.object, "dl:goal")?);
                }
                DL_TEXT => {
                    if text.is_some() {
                        return Err(dup("dl:text"));
                    }
                    text = Some(string_const(&t.object, "dl:text")?);
                }
                DL_PROGRAM => {
                    if program.is_some() {
                        return Err(dup("dl:program"));
                    }
                    program = Some(match &t.object {
                        TermPattern::NamedNode(n) => TermOrVar::Const(terms::named_node(n)),
                        _ => return Err(shape("dl:program needs a program IRI constant")),
                    });
                }
                DL_VALID_FROM | DL_VALID_TO => {
                    let TermPattern::Variable(v) = &t.object else {
                        return Err(shape(format!("<{}> binds a variable", p.as_str())));
                    };
                    let slot = if p.as_str() == DL_VALID_FROM {
                        &mut from
                    } else {
                        &mut to
                    };
                    if slot.is_some() {
                        return Err(dup(p.as_str()));
                    }
                    *slot = Some(self.var(v.as_str()));
                }
                other => return Err(shape(format!("unknown property <{other}>"))),
            }
        }
        let goal = goal.ok_or_else(|| shape("dl:goal is missing"))?;
        let program = match (program, text) {
            (Some(i), None) => DlProgramRef::Iri(i),
            (None, Some(t)) => DlProgramRef::Text(t),
            (Some(_), Some(_)) => return Err(shape("give dl:program or dl:text, not both")),
            (None, None) => return Err(shape("dl:program or dl:text is missing")),
        };
        let mut call = DatalogPattern::new(program, goal.clone(), view);
        for name in tm_ir::goal_variables(&goal) {
            let v = self.var(&name);
            call.outputs.push((name, v));
        }
        call.valid = match (from, to) {
            (None, None) => None,
            // an unrequested end still takes a column of its own
            (Some(f), Some(t)) => Some((f, t)),
            (Some(f), None) => Some((f, self.vars.fresh("dlTo"))),
            (None, Some(t)) => Some((self.vars.fresh("dlFrom"), t)),
        };
        Ok(Op::Datalog(call))
    }
}

fn string_const(t: &TermPattern, what: &str) -> Result<String> {
    match t {
        TermPattern::Literal(l) => match terms::literal(l)? {
            Value::Str(s) => Ok(s),
            _ => Err(shape(format!("{what} needs a plain string literal"))),
        },
        TermPattern::Variable(_) => Err(shape(format!(
            "{what} must be a constant, not a variable (a call runs once, never per row)"
        ))),
        _ => Err(shape(format!("{what} needs a string literal"))),
    }
}
