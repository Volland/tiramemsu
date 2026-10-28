//! N3 reasoning through the bridge (feature `n3`): the read operations
//! `reason` and `ruleSetInfo`, the transaction op `storeRules` and the database
//! operation `materialize`.
//!
//! A rule set is named by a term; a plain string names an IRI (rule sets are
//! IRIs in practice), any other term form is read as usual. Conclusion terms
//! are bridge terms, plus `{"blank": label}` for a node a rule invented,
//! `{"list": [...]}` and `{"formula": "<N3 text>"}`.
// @lat: [[bindings#JSON Bridge#N3 Reasoning]]

use serde_json::{json, Value as J};
use tiramemsu::{
    Conclusion, Db, Eid, MaterializeOptions, MaterializeReport, Monotonicity, N3Term, ObjectId,
    Premise, ReasonLimit, ReasonLimits, ReasonOptions, ReasonResult, ReasonScope, ReasonStatus,
    Rules, Tag, TimeMode, Tx, TxN3, Valid, Value, View,
};

use crate::read::report_json;
use crate::value::{time_from_json, value_from_json, value_to_json};
use crate::{arg, Res};

/// A rule-set term: a plain string is an IRI.
pub fn rule_set_from_json(j: &J) -> Res<Value> {
    match j {
        J::String(s) => Ok(Value::iri(s.clone())),
        J::Null => Err(arg("`ruleSet` is required")),
        other => value_from_json(other),
    }
}

/// `rules` (N3 text) or `ruleSet` (a stored rule set), exactly one.
fn rules_arg(args: &J) -> Res<Rules> {
    let get = |k: &str| args.get(k).filter(|v| !v.is_null());
    match (get("rules"), get("ruleSet")) {
        (Some(J::String(text)), None) => Ok(Rules::Text(text.clone())),
        (Some(_), None) => Err(arg("`rules` must be N3 text")),
        (None, Some(rs)) => Ok(Rules::Stored(rule_set_from_json(rs)?)),
        _ => Err(arg("give exactly one of `rules` (N3 text) and `ruleSet`")),
    }
}

fn terms(j: &J, what: &str) -> Res<Vec<Value>> {
    j.as_array()
        .ok_or_else(|| arg(format!("`{what}` must be a list of terms")))?
        .iter()
        .map(value_from_json)
        .collect()
}

/// `"all"` (or absent), `{"graphs": [terms]}` or `{"seeds": {"nodes": [terms], "hops": n}}`.
fn scope_arg(j: Option<&J>) -> Res<ReasonScope> {
    match j {
        None | Some(J::Null) => Ok(ReasonScope::All),
        Some(J::String(s)) if s == "all" => Ok(ReasonScope::All),
        Some(J::Object(o)) if o.len() == 1 && o.contains_key("graphs") => {
            Ok(ReasonScope::Graphs(terms(&o["graphs"], "graphs")?))
        }
        Some(J::Object(o)) if o.len() == 1 && o.contains_key("seeds") => {
            let s = &o["seeds"];
            let hops = match s.get("hops") {
                None | Some(J::Null) => 1,
                Some(h) => h
                    .as_u64()
                    .and_then(|h| u32::try_from(h).ok())
                    .ok_or_else(|| arg("`hops` must be a non-negative integer"))?,
            };
            if let Some(k) = s
                .as_object()
                .and_then(|m| m.keys().find(|k| *k != "nodes" && *k != "hops"))
            {
                return Err(arg(format!("unknown seeds option {k:?}")));
            }
            Ok(ReasonScope::Seeds {
                nodes: terms(s.get("nodes").unwrap_or(&J::Null), "nodes")?,
                hops,
            })
        }
        Some(_) => Err(arg(
            "`scope` must be \"all\", {\"graphs\": [...]} or {\"seeds\": {\"nodes\": [...], \"hops\": n}}",
        )),
    }
}

/// `{"from": time, "to": time}` (either bound may be left out); absent is unbounded.
fn window_arg(j: Option<&J>) -> Res<Valid> {
    match j {
        None | Some(J::Null) => Ok(Valid::ALWAYS),
        Some(J::Object(o)) => {
            if let Some(k) = o.keys().find(|k| *k != "from" && *k != "to") {
                return Err(arg(format!("unknown window bound {k:?}")));
            }
            let bound = |k: &str| o.get(k).map_or(Ok(None), time_from_json);
            Ok(Valid {
                from: bound("from")?,
                to: bound("to")?,
            })
        }
        Some(_) => Err(arg("`window` must be {\"from\": time, \"to\": time}")),
    }
}

fn count(args: &J, key: &str) -> Res<Option<usize>> {
    match args.get(key) {
        None | Some(J::Null) => Ok(None),
        Some(n) => n
            .as_u64()
            .map(|n| Some(n as usize))
            .ok_or_else(|| arg(format!("`{key}` must be a non-negative integer"))),
    }
}

fn flag(args: &J, key: &str) -> Res<bool> {
    match args.get(key) {
        None | Some(J::Null) => Ok(false),
        Some(J::Bool(b)) => Ok(*b),
        Some(_) => Err(arg(format!("`{key}` must be a boolean"))),
    }
}

/// `{"maxIterations", "maxMatchSteps", "maxBackwardDepth", "maxTermBytes"}`.
fn limits_arg(j: Option<&J>) -> Res<ReasonLimits> {
    let mut l = ReasonLimits::default();
    match j {
        None | Some(J::Null) => {}
        Some(J::Object(o)) => {
            for (k, v) in o {
                let n = v
                    .as_u64()
                    .ok_or_else(|| arg(format!("limit {k} must be a non-negative integer")))?
                    as usize;
                match k.as_str() {
                    "maxIterations" => l.max_iterations = Some(n),
                    "maxMatchSteps" => l.max_match_steps = Some(n),
                    "maxBackwardDepth" => l.max_backward_depth = Some(n),
                    "maxTermBytes" => l.max_term_bytes = Some(n),
                    other => return Err(arg(format!("unknown limit {other:?}"))),
                }
            }
        }
        Some(_) => Err(arg("`limits` must be an object"))?,
    }
    Ok(l)
}

/// The options of a `reason` call.
pub fn reason_options(args: &J) -> Res<ReasonOptions> {
    let mut o = ReasonOptions {
        scope: scope_arg(args.get("scope"))?,
        window: window_arg(args.get("window"))?,
        proof_n3: flag(args, "proofN3")?,
        limits: limits_arg(args.get("limits"))?,
        ..ReasonOptions::default()
    };
    o.time = match args.get("time").and_then(J::as_str) {
        None => TimeMode::Pointwise,
        Some("pointwise") => TimeMode::Pointwise,
        Some("atemporal") => TimeMode::Atemporal,
        Some(other) => {
            return Err(arg(format!(
                "time must be \"pointwise\" or \"atemporal\", not {other:?}"
            )))
        }
    };
    if let Some(n) = count(args, "maxSegments")? {
        o.max_segments = n;
    }
    Ok(o)
}

/// The JSON form of a conclusion term.
pub fn term_json(t: &N3Term) -> J {
    match t {
        N3Term::Value(v) => value_to_json(v),
        N3Term::Blank(b) => json!({ "blank": b }),
        N3Term::List(items) => json!({ "list": items.iter().map(term_json).collect::<Vec<_>>() }),
        N3Term::Formula(text) => json!({ "formula": text }),
    }
}

fn bound(b: Option<i64>) -> J {
    b.map_or(J::Null, |v| json!(v))
}

/// `{s, p, o, validFrom, validTo, proof: {ruleSet, rule, premises: [{eid} | {conclusion}]}}`.
pub fn conclusion_json(c: &Conclusion) -> J {
    json!({
        "s": term_json(&c.s),
        "p": term_json(&c.p),
        "o": term_json(&c.o),
        "validFrom": bound(c.valid.from),
        "validTo": bound(c.valid.to),
        "proof": {
            "ruleSet": c.proof.rule_set.as_ref().map_or(J::Null, value_to_json),
            "rule": c.proof.rule,
            "premises": c.proof.premises.iter().map(|p| match p {
                Premise::Statement(e) => json!({ "eid": e.n() }),
                Premise::Conclusion(i) => json!({ "conclusion": i }),
            }).collect::<Vec<_>>(),
        },
    })
}

fn limit_name(l: ReasonLimit) -> &'static str {
    match l {
        ReasonLimit::Segments => "segments",
        ReasonLimit::Iterations => "iterations",
        ReasonLimit::MatchSteps => "matchSteps",
        ReasonLimit::BackwardDepth => "backwardDepth",
        ReasonLimit::BackwardSolutions => "backwardSolutions",
        ReasonLimit::TermBytes => "termBytes",
        ReasonLimit::TermDepth => "termDepth",
    }
}

/// `("complete" | "incomplete", [limit names])`.
fn status_json(s: &ReasonStatus) -> (J, J) {
    match s {
        ReasonStatus::Complete => (json!("complete"), json!([])),
        ReasonStatus::Incomplete { limits } => (
            json!("incomplete"),
            json!(limits.iter().map(|l| limit_name(*l)).collect::<Vec<_>>()),
        ),
    }
}

/// The JSON form of a [`ReasonResult`]: `{conclusions, t, now, loaded, pruned,
/// segments, status, limits, errors, contradiction, ruleSet, ruleSetStatement,
/// proofN3}`.
pub fn reason_json(r: &ReasonResult) -> J {
    let (status, limits) = status_json(&r.status);
    json!({
        "conclusions": r.conclusions.iter().map(conclusion_json).collect::<Vec<_>>(),
        "t": r.t,
        "now": r.now,
        "loaded": r.loaded,
        "pruned": r.pruned,
        "segments": r.segments,
        "status": status,
        "limits": limits,
        "errors": r.errors,
        "contradiction": r.contradiction.as_ref().map_or(J::Null, |c| json!({
            "rule": c.rule,
            "instance": c.instance,
            "validFrom": bound(c.valid.from),
            "validTo": bound(c.valid.to),
        })),
        "ruleSet": r.rule_set.as_ref().map_or(J::Null, value_to_json),
        "ruleSetStatement": r.rule_set_statement.map_or(J::Null, |e| json!(e.n())),
        "proofN3": r.proof_n3,
    })
}

/// The read `reason`: `rules` or `ruleSet`, `scope`, `window`, `time`,
/// `maxSegments`, `proofN3` and `limits` on the call's view.
// @tg: implements:: [[openspec:n3-reasoning#Reasoning surfaces]] {since: "2026-10-28"}
pub fn reason(view: &View<'_>, args: &J) -> Res<J> {
    let rules = rules_arg(args)?;
    let opts = reason_options(args)?;
    Ok(reason_json(&view.reason(rules, &opts)?))
}

fn monotonicity_json(m: &Monotonicity) -> J {
    match m {
        Monotonicity::Monotonic => json!({ "kind": "monotonic" }),
        Monotonicity::NonMonotonic { rule, builtin } => {
            json!({ "kind": "nonMonotonic", "rule": rule, "builtin": builtin })
        }
        Monotonicity::ClockDependent { rule, builtin } => {
            json!({ "kind": "clockDependent", "rule": rule, "builtin": builtin })
        }
        Monotonicity::UntracedPremises { rule, builtin } => {
            json!({ "kind": "untracedPremises", "rule": rule, "builtin": builtin })
        }
    }
}

/// The live `n3:source` statement of `rule_set` in `view` and its text.
fn source(view: &View<'_>, rule_set: &Value) -> Res<Option<(Eid, String)>> {
    let source = Value::iri(tiramemsu::n3_engine::vocab::N3_SOURCE);
    let (Some(s), Some(p)) = (view.encode(rule_set)?, view.encode(&source)?) else {
        return Ok(None);
    };
    // the newest version wins in a history view
    let Some(row) = view.triples(Some(s), Some(p), None)?.pop() else {
        return Ok(None);
    };
    Ok(match view.decode(row.o)? {
        Value::Str(text) => Some((row.eid, text)),
        _ => None,
    })
}

/// A stored rule set as `view` sees it: `{ruleSet, statement, text, rules,
/// monotonicity: {kind, rule?, builtin?}, predicates, variablePredicate}`.
pub fn rule_set_json(view: &View<'_>, rule_set: &Value) -> Res<J> {
    let info = view.rule_set_info(rule_set)?;
    let (statement, text) =
        source(view, rule_set)?.map_or((J::Null, J::Null), |(e, t)| (json!(e.n()), json!(t)));
    Ok(json!({
        "ruleSet": value_to_json(rule_set),
        "statement": statement,
        "text": text,
        "rules": info.rules,
        "monotonicity": monotonicity_json(&info.monotonicity),
        "predicates": info.predicates,
        "variablePredicate": info.variable_predicate,
    }))
}

/// The read `ruleSetInfo`: `ruleSet`.
// @tg: implements:: [[openspec:n3-rule-sets#Rule-set surfaces]] {since: "2026-10-28"}
pub fn rule_set_info(view: &View<'_>, args: &J) -> Res<J> {
    let rs = rule_set_from_json(args.get("ruleSet").unwrap_or(&J::Null))?;
    rule_set_json(view, &rs)
}

/// The transaction op `storeRules`: `{ruleSet, n3}` to `{eid}`, the live
/// `n3:source` statement (the existing one for equal text).
// @tg: implements:: [[openspec:n3-rule-sets#Rule-set surfaces]] {since: "2026-10-28"}
pub fn store_rules(tx: &mut Tx<'_>, op: &J) -> Res<Eid> {
    let rs = rule_set_from_json(op.get("ruleSet").unwrap_or(&J::Null))?;
    let text = op
        .get("n3")
        .and_then(J::as_str)
        .ok_or_else(|| arg("`n3` must be N3 text"))?;
    Ok(tx.store_rules(rs, text)?)
}

/// The options of a `materialize` call.
pub fn materialize_options(args: &J) -> Res<MaterializeOptions> {
    let mut o = MaterializeOptions {
        scope: scope_arg(args.get("scope"))?,
        window: window_arg(args.get("window"))?,
        limits: limits_arg(args.get("limits"))?,
        require_complete: flag(args, "requireComplete")?,
        dry_run: flag(args, "dryRun")?,
        ..MaterializeOptions::default()
    };
    if let Some(n) = count(args, "maxSegments")? {
        o.max_segments = n;
    }
    Ok(o)
}

/// A Skolem node's term: an anonymous node, decoded from its id.
fn skolem_json(db: &Db, id: ObjectId) -> Res<J> {
    if matches!(id.tag(), Ok(Tag::Node)) {
        return Ok(value_to_json(&Value::Node(id.unsigned_payload())));
    }
    Ok(value_to_json(&db.now().decode(id)?))
}

/// The JSON form of a [`MaterializeReport`]: `{ruleSet, rulesStatement,
/// reasonedAt, report, derived, existing, widened: [{old, new}], skolems,
/// staleSupport, unstorable, status, limits, dryRun}`.
pub fn materialize_json(db: &Db, r: &MaterializeReport) -> Res<J> {
    let ids = |v: &[Eid]| v.iter().map(|e| e.n()).collect::<Vec<_>>();
    let (status, limits) = status_json(&r.status);
    Ok(json!({
        "ruleSet": value_to_json(&r.rule_set),
        "rulesStatement": r.rules_statement.n(),
        "reasonedAt": r.reasoned_at,
        "report": r.commit.as_ref().map_or(J::Null, report_json),
        "derived": ids(&r.derived),
        "existing": ids(&r.existing),
        "widened": r.widened.iter().map(|(o, n)| json!({ "old": o.n(), "new": n.n() })).collect::<Vec<_>>(),
        "skolems": r.skolems.iter().map(|id| skolem_json(db, *id)).collect::<Res<Vec<_>>>()?,
        "staleSupport": r.stale_support.iter().map(conclusion_json).collect::<Vec<_>>(),
        "unstorable": r.unstorable.iter().map(conclusion_json).collect::<Vec<_>>(),
        "status": status,
        "limits": limits,
        "dryRun": r.dry_run,
    }))
}

/// The database operation `materialize`: `ruleSet`, `scope`, `window`,
/// `maxSegments`, `limits`, `requireComplete` and `dryRun`.
// @tg: implements:: [[openspec:n3-materialization#Materialization surfaces]] {since: "2026-10-28"}
pub fn materialize(db: &Db, args: &J) -> Res<J> {
    let rs = rule_set_from_json(args.get("ruleSet").unwrap_or(&J::Null))?;
    let opts = materialize_options(args)?;
    let r = db.materialize(&rs, &opts)?;
    materialize_json(db, &r)
}
