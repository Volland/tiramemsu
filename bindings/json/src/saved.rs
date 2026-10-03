//! Saved answers: `saveAnswer`, `savedAnswer`, `savedAnswers`,
//! `checkSavedAnswers`, `refreshAnswer` and `deleteSavedAnswer` (see
//! [`tiramemsu::SavedAnswer`]).

use serde_json::{json, Map, Value as J};
use tiramemsu::{
    Db, Invalidation, QueryBudget, QueryLanguage, SavedAnswer, SavedQuery, SparqlResult, TimeRef,
    TxSel, ValidSel, ViewSpec,
};

use crate::read::{event_json, sparql_json, view_from_json};
use crate::value::params_from_json;
use crate::{arg, Res};

fn name_arg(args: &J) -> Res<&str> {
    args.get("name")
        .and_then(J::as_str)
        .ok_or_else(|| arg("`name` must be a string"))
}

/// The view descriptor in the bridge's JSON form.
fn view_json(v: &ViewSpec) -> J {
    let mut o = Map::new();
    let kind = match v.tx {
        TxSel::Now => "now",
        TxSel::History => "history",
        TxSel::AsOf(at) => {
            match at {
                TimeRef::Tx(t) => o.insert("tx".into(), json!(t)),
                TimeRef::Instant(ms) => o.insert("instant".into(), json!(ms)),
            };
            "asOf"
        }
    };
    o.insert("kind".into(), json!(kind));
    if let ValidSel::At(ms) = v.valid {
        o.insert("validAt".into(), json!(ms));
    }
    J::Object(o)
}

/// An invalidation as JSON: `{name, status, cause, t, event}`.
pub(crate) fn invalidation_json(i: &Invalidation) -> J {
    json!({
        "name": i.name,
        "status": i.status.name(),
        "cause": i.cause.name(),
        "t": i.t.map(|t| t.0),
        "event": i.event.as_ref().map_or(J::Null, event_json),
    })
}

/// A saved answer as JSON. `result` has the shape the live `sparql` or `cypher`
/// call returns.
pub(crate) fn answer_json(a: &SavedAnswer) -> Res<J> {
    let result = match a.solutions()? {
        Some(s) => sparql_json(&SparqlResult::Solutions(s)),
        None => match a.boolean() {
            Some(b) => sparql_json(&SparqlResult::Boolean(b)),
            None => a.result.clone(),
        },
    };
    Ok(json!({
        "name": a.name,
        "language": a.query.language.name(),
        "text": a.query.text,
        "params": a.query.params.iter().map(|(k, v)| (k.clone(), v.to_json())).collect::<Map<_, _>>(),
        "view": view_json(&a.query.view),
        "vocab": a.vocab,
        "prefixes": a.prefixes.iter().map(|(n, i)| json!([n, i])).collect::<Vec<_>>(),
        "result": result,
        "dependencies": a.dependencies.iter().map(|e| e.n()).collect::<Vec<_>>(),
        "coverage": a.coverage.iter().map(|c| c.name()).collect::<Vec<_>>(),
        "checkpoint": a.checkpoint.0,
        "cursor": a.cursor.0,
        "evaluatedAt": a.evaluated_at,
        "revision": a.revision,
        "status": a.status.name(),
        "invalidation": a.invalidation.as_ref().map_or(J::Null, invalidation_json),
        "error": a.error,
    }))
}

/// Runs one saved-answer operation; `budget` bounds `saveAnswer` and
/// `refreshAnswer`.
pub fn run(db: &Db, op: &str, args: &J, budget: Option<&QueryBudget>) -> Res<J> {
    match op {
        "saveAnswer" => {
            let language = match args.get("language") {
                None | Some(J::Null) => QueryLanguage::Sparql,
                Some(l) => l
                    .as_str()
                    .and_then(QueryLanguage::from_name)
                    .ok_or_else(|| arg("`language` must be \"sparql\" or \"cypher\""))?,
            };
            let text = args
                .get("text")
                .and_then(J::as_str)
                .ok_or_else(|| arg("`text` must be a string"))?;
            let view = view_from_json(db, args.get("view").unwrap_or(&J::Null))?.spec();
            let q = SavedQuery {
                language,
                text: text.to_string(),
                params: params_from_json(args.get("params").unwrap_or(&J::Null))?,
                view,
            };
            answer_json(&db.save_answer_with(name_arg(args)?, &q, budget)?)
        }
        "savedAnswer" => match db.saved_answer(name_arg(args)?)? {
            Some(a) => answer_json(&a),
            None => Ok(J::Null),
        },
        "savedAnswers" => Ok(J::Array(
            db.saved_answers()?
                .iter()
                .map(answer_json)
                .collect::<Res<_>>()?,
        )),
        "checkSavedAnswers" => Ok(J::Array(
            db.check_saved_answers()?
                .iter()
                .map(invalidation_json)
                .collect(),
        )),
        "refreshAnswer" => answer_json(&db.refresh_answer_with(name_arg(args)?, budget)?),
        "deleteSavedAnswer" => Ok(json!({ "deleted": db.delete_saved_answer(name_arg(args)?)? })),
        other => Err(arg(format!("unknown operation {other:?}"))),
    }
}
