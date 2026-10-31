//! `SERVICE <urn:tiramemsu:dl:query> { [] dl:goal "…" ; dl:program <iri> | dl:text "…" ;
//! dl:validFrom ?f ; dl:validTo ?t }` → [`Op::Datalog`] (`lat.md/query#Datalog Calls`),
//! and the same call of an ELPI goal under `SERVICE <urn:tiramemsu:elpi:query>` with
//! the `elpi:` properties (`lat.md/elpi#Own Calls`).
//!
//! The front end has no Datalog parser: the goal's variables are read textually
//! ([`tm_ir::goal_variables`]) and each binds the SPARQL variable of the same name.

use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern};
use tm_core::vocab::{
    DL_GOAL, DL_PROGRAM, DL_TEXT, DL_VALID_FROM, DL_VALID_TO, ELPI_GOAL, ELPI_PROGRAM, ELPI_TEXT,
    ELPI_VALID_FROM, ELPI_VALID_TO,
};
use tm_core::{Error, Result, Value};
use tm_ir::{DatalogPattern, DlProgramRef, GoalLanguage, Op, TermOrVar, View};

use super::Lowerer;
use crate::terms;

fn shape(msg: impl std::fmt::Display) -> Error {
    Error::DatalogCallShape {
        message: msg.to_string(),
    }
}

impl Lowerer<'_> {
    /// Lowers the group of a `SERVICE <urn:tiramemsu:dl:query>` (or, for
    /// `GoalLanguage::Elpi`, of a `SERVICE <urn:tiramemsu:elpi:query>`).
    // @tg: implements:: [[openspec:datalog-query-calls#SPARQL goal service]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:elpi-query-calls#ELPI goal calls]] {since: "2026-10-31"}
    pub(super) fn datalog_service(
        &mut self,
        inner: &GraphPattern,
        view: View,
        language: GoalLanguage,
    ) -> Result<Op> {
        let (p_goal, p_program, p_text, p_from, p_to, ns) = match language {
            GoalLanguage::Datalog => (
                DL_GOAL,
                DL_PROGRAM,
                DL_TEXT,
                DL_VALID_FROM,
                DL_VALID_TO,
                "dl",
            ),
            GoalLanguage::Elpi => (
                ELPI_GOAL,
                ELPI_PROGRAM,
                ELPI_TEXT,
                ELPI_VALID_FROM,
                ELPI_VALID_TO,
                "elpi",
            ),
        };
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
                return Err(shape(format!(
                    "a call's property is {ns}:goal, {ns}:program, {ns}:text, {ns}:validFrom or {ns}:validTo"
                )));
            };
            let dup = |name: &str| shape(format!("{name} given twice"));
            let p = p.as_str();
            if p == p_goal {
                if goal.is_some() {
                    return Err(dup(&format!("{ns}:goal")));
                }
                goal = Some(string_const(&t.object, &format!("{ns}:goal"))?);
            } else if p == p_text {
                if text.is_some() {
                    return Err(dup(&format!("{ns}:text")));
                }
                text = Some(string_const(&t.object, &format!("{ns}:text"))?);
            } else if p == p_program {
                if program.is_some() {
                    return Err(dup(&format!("{ns}:program")));
                }
                program = Some(match &t.object {
                    TermPattern::NamedNode(n) => TermOrVar::Const(terms::named_node(n)),
                    _ => return Err(shape(format!("{ns}:program needs a program IRI constant"))),
                });
            } else if p == p_from || p == p_to {
                let TermPattern::Variable(v) = &t.object else {
                    return Err(shape(format!("<{p}> binds a variable")));
                };
                let slot = if p == p_from { &mut from } else { &mut to };
                if slot.is_some() {
                    return Err(dup(p));
                }
                *slot = Some(self.var(v.as_str()));
            } else {
                return Err(shape(format!("unknown property <{p}>")));
            }
        }
        let goal = goal.ok_or_else(|| shape(format!("{ns}:goal is missing")))?;
        let program = match (program, text) {
            (Some(i), None) => DlProgramRef::Iri(i),
            (None, Some(t)) => DlProgramRef::Text(t),
            (Some(_), Some(_)) => {
                return Err(shape(format!("give {ns}:program or {ns}:text, not both")))
            }
            (None, None) => return Err(shape(format!("{ns}:program or {ns}:text is missing"))),
        };
        let mut call = DatalogPattern::new(program, goal.clone(), view);
        call.language = language;
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
