//! Datalog through the bridge (feature `datalog`): the read operations
//! `datalog`, `datalogReason`, `programInfo`, `explainDatalog` and `check`, and
//! the transaction op `storeProgram`.
//!
//! A call names its program with exactly one of `program` (Datalog text) and
//! `stored` (a program node; a plain string is an IRI). Result terms are
//! bridge terms, plus `{"invented": n}` for a node a rule invented.
// @lat: [[bindings#JSON Bridge#Datalog]]

use serde_json::{json, Map, Value as J};
use tiramemsu::{
    CheckResult, DatalogAccess, DatalogConclusion, DatalogFact, DatalogLimits, DatalogOptions,
    DatalogPremise, DatalogProgram, DatalogProof, DatalogResult, DatalogScope, DatalogStatus,
    DatalogTerm, DatalogTime, Eid, ProgramInfo, Tx, TxDatalog, Valid, Value, View,
};

use crate::value::{time_from_json, value_from_json, value_to_json};
use crate::{arg, Res};

/// A program node: a plain string is an IRI.
pub fn node_from_json(j: &J) -> Res<Value> {
    match j {
        J::String(s) => Ok(Value::iri(s.clone())),
        J::Null => Err(arg("a program node is required")),
        other => value_from_json(other),
    }
}

/// `program` (text) or `stored` (a program node), exactly one.
fn program_arg(args: &J) -> Res<DatalogProgram> {
    let get = |k: &str| args.get(k).filter(|v| !v.is_null());
    match (get("program"), get("stored")) {
        (Some(J::String(text)), None) => Ok(DatalogProgram::Text(text.clone())),
        (Some(_), None) => Err(arg("`program` must be Datalog text")),
        (None, Some(n)) => Ok(DatalogProgram::Stored(node_from_json(n)?)),
        _ => Err(arg("give exactly one of `program` (Datalog text) and `stored`")),
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
fn scope_arg(j: Option<&J>) -> Res<DatalogScope> {
    match j {
        None | Some(J::Null) => Ok(DatalogScope::All),
        Some(J::String(s)) if s == "all" => Ok(DatalogScope::All),
        Some(J::Object(o)) if o.len() == 1 && o.contains_key("graphs") => {
            Ok(DatalogScope::Graphs(terms(&o["graphs"], "graphs")?))
        }
        Some(J::Object(o)) if o.len() == 1 && o.contains_key("seeds") => {
            let s = &o["seeds"];
            let hops = match s.get("hops") {
                None | Some(J::Null) => 1,
                Some(h) => h
                    .as_u64()
                    .ok_or_else(|| arg("`hops` must be a non-negative integer"))? as usize,
            };
            Ok(DatalogScope::Seeds {
                nodes: terms(s.get("nodes").unwrap_or(&J::Null), "nodes")?,
                hops,
            })
        }
        Some(_) => Err(arg(
            "`scope` must be \"all\", {\"graphs\": [...]} or {\"seeds\": {\"nodes\": [...], \"hops\": n}}",
        )),
    }
}

fn window_arg(j: Option<&J>) -> Res<Option<Valid>> {
    match j {
        None | Some(J::Null) => Ok(None),
        Some(J::Object(o)) => {
            if let Some(k) = o.keys().find(|k| *k != "from" && *k != "to") {
                return Err(arg(format!("unknown window bound {k:?}")));
            }
            let bound = |k: &str| o.get(k).map_or(Ok(None), time_from_json);
            Ok(Some(Valid {
                from: bound("from")?,
                to: bound("to")?,
            }))
        }
        Some(_) => Err(arg("`window` must be {\"from\": time, \"to\": time}")),
    }
}

fn flag(args: &J, key: &str, default: bool) -> Res<bool> {
    match args.get(key) {
        None | Some(J::Null) => Ok(default),
        Some(J::Bool(b)) => Ok(*b),
        Some(_) => Err(arg(format!("`{key}` must be a boolean"))),
    }
}

/// `{"maxIterations", "maxFacts", "maxFragments", "maxNewValues"}`.
fn limits_arg(j: Option<&J>) -> Res<DatalogLimits> {
    let mut l = DatalogLimits::default();
    match j {
        None | Some(J::Null) => {}
        Some(J::Object(o)) => {
            for (k, v) in o {
                let n = v
                    .as_u64()
                    .ok_or_else(|| arg(format!("limit {k} must be a non-negative integer")))?;
                match k.as_str() {
                    "maxIterations" => l.max_iterations = n,
                    "maxFacts" => l.max_facts = Some(n),
                    "maxFragments" => l.max_fragments = Some(n),
                    "maxNewValues" => l.max_new_values = Some(n),
                    other => return Err(arg(format!("unknown limit {other:?}"))),
                }
            }
        }
        Some(_) => Err(arg("`limits` must be an object"))?,
    }
    Ok(l)
}

/// The options of a Datalog call.
pub fn options(args: &J) -> Res<DatalogOptions> {
    let mut o = DatalogOptions {
        scope: scope_arg(args.get("scope"))?,
        window: window_arg(args.get("window"))?,
        limits: limits_arg(args.get("limits"))?,
        demand: flag(args, "demand", true)?,
        proof: flag(args, "proof", false)?,
        include_relations: flag(args, "includeRelations", false)?,
        require_complete: flag(args, "requireComplete", false)?,
        ..DatalogOptions::default()
    };
    o.time = match args.get("time").and_then(J::as_str) {
        None | Some("pointwise") => DatalogTime::Pointwise,
        Some("atemporal") => DatalogTime::Atemporal,
        Some(other) => {
            return Err(arg(format!(
                "time must be \"pointwise\" or \"atemporal\", not {other:?}"
            )))
        }
    };
    o.access = match args.get("access").and_then(J::as_str) {
        None | Some("auto") => DatalogAccess::Auto,
        Some("probe") => DatalogAccess::Probe,
        Some("load") => DatalogAccess::Load,
        Some(other) => return Err(arg(format!("access must be auto, probe or load, not {other:?}"))),
    };
    if let Some(n) = args.get("parallelThreshold").filter(|v| !v.is_null()) {
        o.parallel_threshold = n
            .as_u64()
            .ok_or_else(|| arg("`parallelThreshold` must be a non-negative integer"))?
            as usize;
    }
    Ok(o)
}

/// The JSON form of a result term.
pub fn term_json(t: &DatalogTerm) -> J {
    match t {
        DatalogTerm::Value(v) => value_to_json(v),
        DatalogTerm::Invented(n) => json!({ "invented": n }),
    }
}

fn bound(b: Option<i64>) -> J {
    b.map_or(J::Null, |v| json!(v))
}

fn valid_json(v: &Valid) -> J {
    json!({ "from": bound(v.from), "to": bound(v.to) })
}

fn proof_json(p: &DatalogProof) -> J {
    json!({
        "rule": p.rule,
        "premises": p.premises.iter().map(|x| match x {
            DatalogPremise::Statement(e) => json!({ "eid": e.n() }),
            DatalogPremise::Fact(i) => json!({ "fact": i }),
        }).collect::<Vec<_>>(),
    })
}

fn fact_json(f: &DatalogFact) -> J {
    json!({
        "relation": f.relation,
        "args": f.args.iter().map(term_json).collect::<Vec<_>>(),
        "valid": f.valid.iter().map(valid_json).collect::<Vec<_>>(),
        "proof": f.proof.as_ref().map_or(J::Null, proof_json),
    })
}

fn status_json(s: &DatalogStatus) -> (J, J) {
    match s {
        DatalogStatus::Complete => (json!("complete"), json!([])),
        DatalogStatus::Incomplete { limits } => (json!("incomplete"), json!(limits)),
    }
}

/// The JSON form of a goal result.
pub fn result_json(r: &DatalogResult) -> J {
    let (status, limits) = status_json(&r.status);
    json!({
        "columns": r.columns,
        "rows": r.rows.iter().map(|row| {
            let mut m = Map::new();
            m.insert("values".into(), json!(row.values.iter().map(term_json).collect::<Vec<_>>()));
            m.insert("valid".into(), json!(row.valid.iter().map(valid_json).collect::<Vec<_>>()));
            if let Some(p) = &row.proof {
                m.insert("proof".into(), json!(p.iter().map(|f| json!({
                    "valid": valid_json(&f.valid),
                    "proof": proof_json(&f.proof),
                })).collect::<Vec<_>>()));
            }
            J::Object(m)
        }).collect::<Vec<_>>(),
        "facts": r.facts.iter().map(fact_json).collect::<Vec<_>>(),
        "t": r.t,
        "now": r.now,
        "strata": r.strata,
        "derived": r.derived,
        "predicates": r.predicates,
        "pruned": r.pruned,
        "demand": r.demand,
        "status": status,
        "limits": limits,
        "program": r.program.as_ref().map_or(J::Null, value_to_json),
        "programStatement": r.program_statement.map_or(J::Null, |e| json!(e.n())),
    })
}

fn conclusion_json(c: &DatalogConclusion) -> J {
    json!({
        "s": term_json(&c.s),
        "p": value_to_json(&c.p),
        "o": term_json(&c.o),
        "validFrom": bound(c.valid.from),
        "validTo": bound(c.valid.to),
        "proof": c.proof.as_ref().map_or(J::Null, proof_json),
    })
}

fn goal_arg(args: &J) -> Res<Option<&str>> {
    match args.get("goal") {
        None | Some(J::Null) => Ok(None),
        Some(J::String(g)) => Ok(Some(g)),
        Some(_) => Err(arg("`goal` must be a Datalog body")),
    }
}

/// The read `datalog`: `program` or `stored`, `goal`, and the options.
// @tg: implements:: [[openspec:datalog-programs#Datalog read surfaces]] {since: "2026-10-28"}
pub fn query(view: &View<'_>, args: &J) -> Res<J> {
    let p = program_arg(args)?;
    let o = options(args)?;
    Ok(result_json(&view.datalog(&p, goal_arg(args)?, &o)?))
}

/// The read `datalogReason`: every conclusion not among the stored statements.
pub fn reason(view: &View<'_>, args: &J) -> Res<J> {
    let p = program_arg(args)?;
    let o = options(args)?;
    let r = view.datalog_reason(&p, &o)?;
    let (status, limits) = status_json(&r.status);
    Ok(json!({
        "conclusions": r.conclusions.iter().map(conclusion_json).collect::<Vec<_>>(),
        "relations": r.relations.iter().map(fact_json).collect::<Vec<_>>(),
        "facts": r.facts.iter().map(fact_json).collect::<Vec<_>>(),
        "t": r.t,
        "now": r.now,
        "predicates": r.predicates,
        "status": status,
        "limits": limits,
        "program": r.program.as_ref().map_or(J::Null, value_to_json),
        "programStatement": r.program_statement.map_or(J::Null, |e| json!(e.n())),
    }))
}

/// The JSON form of program information.
pub fn info_json(i: &ProgramInfo) -> J {
    let a = &i.analysis;
    json!({
        "program": i.program.as_ref().map_or(J::Null, value_to_json),
        "statement": i.statement.map_or(J::Null, |e| json!(e.n())),
        "text": i.text,
        "rules": a.rules,
        "goals": a.goals,
        "constraints": a.constraints.iter().map(|(n, s)| json!({ "name": n, "severity": s.name() })).collect::<Vec<_>>(),
        "strata": a.strata.iter().map(|s| json!({
            "relations": s.relations,
            "recursive": s.recursive,
            "monotonic": s.monotonic,
        })).collect::<Vec<_>>(),
        "predicates": a.predicates,
        "variablePredicate": a.variable_predicate,
        "metricRecursion": a.metric_recursion,
        "inventionRecursion": a.invention_recursion,
    })
}

/// The read `programInfo`: `program` or `stored`.
pub fn program_info(view: &View<'_>, args: &J) -> Res<J> {
    let p = program_arg(args)?;
    Ok(info_json(&view.program_info(&p)?))
}

/// The read `explainDatalog`: `{plan, result}`.
pub fn explain(view: &View<'_>, args: &J) -> Res<J> {
    let p = program_arg(args)?;
    let o = options(args)?;
    let e = view.explain_datalog(&p, goal_arg(args)?, &o)?;
    Ok(json!({ "plan": e.plan, "result": result_json(&e.result) }))
}

/// The JSON form of a check.
pub fn check_json(r: &CheckResult) -> J {
    let (status, limits) = status_json(&r.status);
    json!({
        "violated": r.violated.iter().map(|c| json!({
            "name": c.name,
            "severity": c.severity.name(),
            "violations": c.violations.iter().map(|v| {
                let b: Map<String, J> = v.bindings.iter().map(|(k, t)| (k.clone(), term_json(t))).collect();
                json!({
                    "bindings": b,
                    "eids": v.eids.iter().map(|e| e.n()).collect::<Vec<_>>(),
                    "valid": v.valid.iter().map(valid_json).collect::<Vec<_>>(),
                })
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "checked": r.checked,
        "t": r.t,
        "now": r.now,
        "status": status,
        "limits": limits,
    })
}

/// The read `check`: `program` or `stored`, `only` (constraint names).
// @tg: implements:: [[openspec:datalog-constraints#Check constraints]] {since: "2026-10-28"}
pub fn check(view: &View<'_>, args: &J) -> Res<J> {
    let p = program_arg(args)?;
    let o = options(args)?;
    let only: Vec<String> = match args.get("only") {
        None | Some(J::Null) => Vec::new(),
        Some(J::Array(a)) => a
            .iter()
            .map(|x| x.as_str().map(String::from).ok_or_else(|| arg("`only` must be a list of names")))
            .collect::<Res<_>>()?,
        Some(_) => return Err(arg("`only` must be a list of names")),
    };
    Ok(check_json(&view.check(&p, &only, &o)?))
}

/// The transaction op `storeProgram`: `{program, text}` to `{eid}`.
pub fn store_program(tx: &mut Tx<'_>, op: &J) -> Res<Eid> {
    let node = node_from_json(op.get("program").unwrap_or(&J::Null))?;
    let text = op
        .get("text")
        .and_then(J::as_str)
        .ok_or_else(|| arg("`text` must be Datalog text"))?;
    Ok(tx.store_program(node, text)?)
}
