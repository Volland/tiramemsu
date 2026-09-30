//! Read operations: views, SPARQL, Cypher, lookups, paths and the event log.

use std::collections::HashMap;

use serde_json::{json, Map, Value as J};
use tiramemsu::{
    BundleFormat, Db, Eid, Event, ObjectId, Op, PathDir, PathMode, PathRow, RdfTerm, RdfTriple,
    SparqlResult, TimeRef, Triple, TxReport, View,
};

use crate::value::{eid_from_json, params_from_json, value_from_json, value_to_json};
use crate::{arg, Res};

/// The view a read runs on, from `{"kind", "tx", "instant", "validAt"}`; `null` is now.
pub fn view_from_json<'a>(db: &'a Db, j: &J) -> Res<View<'a>> {
    let get = |k: &str| j.get(k).filter(|v| !v.is_null());
    let kind = j.get("kind").and_then(J::as_str).unwrap_or("now");
    let view = match kind {
        "now" => db.now(),
        "history" => db.history(),
        "asOf" => match (get("tx"), get("instant")) {
            (Some(t), None) => db.as_of(TimeRef::Tx(
                t.as_u64()
                    .ok_or_else(|| arg("tx must be a non-negative integer"))?,
            )),
            (None, Some(i)) => db.as_of(TimeRef::Instant(instant(i)?)),
            _ => return Err(arg("an asOf view needs exactly one of tx and instant")),
        },
        other => return Err(arg(format!("unknown view kind {other:?}"))),
    };
    Ok(match get("validAt") {
        Some(v) => view.valid_at(instant(v)?),
        None => view,
    })
}

fn instant(j: &J) -> Res<i64> {
    crate::value::time_from_json(j)?.ok_or_else(|| arg("a time is required"))
}

/// Runs one read operation on `view`.
pub fn run(view: &View<'_>, op: &str, args: &J) -> Res<J> {
    match op {
        "sparql" => {
            let text = str_arg(args, "text")?;
            Ok(sparql_json(&view.sparql(text)?))
        }
        "cypher" => {
            let text = str_arg(args, "text")?;
            let params = params_from_json(args.get("params").unwrap_or(&J::Null))?;
            Ok(view.cypher(text, &params)?.to_json())
        }
        "triples" => {
            let ids = [
                opt_id(view, args, "s")?,
                opt_id(view, args, "p")?,
                opt_id(view, args, "o")?,
            ];
            // A term that is not stored cannot match anything.
            if ids.iter().any(|i| matches!(i, Lookup::Missing)) {
                return Ok(json!([]));
            }
            let [s, p, o] = ids.map(Lookup::id);
            let rows = view.triples(s, p, o)?;
            Ok(J::Array(
                rows.iter()
                    .map(|t| triple_json(view, t))
                    .collect::<Res<_>>()?,
            ))
        }
        "path" => {
            let start = match required_id(view, args, "start")? {
                Some(s) => s,
                None => return Ok(json!([])),
            };
            let mode = match args.get("mode").and_then(J::as_str).unwrap_or("reach") {
                "reach" => PathMode::Reachability,
                "trail" => PathMode::Trail,
                "anyShortest" => PathMode::AnyShortest,
                "allShortest" => PathMode::AllShortest,
                other => return Err(arg(format!("unknown path mode {other:?}"))),
            };
            let max = args
                .get("maxHops")
                .and_then(J::as_u64)
                .map_or(u32::MAX, |n| n.min(u32::MAX as u64) as u32);
            let rows = view.path(start, str_arg(args, "path")?, mode, max)?;
            Ok(J::Array(
                rows.iter()
                    .map(|r| path_row_json(view, r))
                    .collect::<Res<_>>()?,
            ))
        }
        "events" => {
            let since = args.get("since").and_then(J::as_u64).unwrap_or(0);
            Ok(J::Array(
                view.events_since(since)?.iter().map(event_json).collect(),
            ))
        }
        "graphs" => Ok(J::Array(
            view.graphs()?
                .into_iter()
                .map(|g| Ok(value_to_json(&view.decode(g)?)))
                .collect::<Res<_>>()?,
        )),
        "graphMembers" => match required_id(view, args, "graph")? {
            Some(g) => Ok(json!(view
                .graph_members(g)?
                .iter()
                .map(|e| e.n())
                .collect::<Vec<_>>())),
            None => Ok(json!([])),
        },
        "values" => match (
            required_id(view, args, "s")?,
            required_id(view, args, "key")?,
        ) {
            (Some(s), Some(k)) => Ok(J::Array(
                view.values(s, k)?
                    .into_iter()
                    .map(|o| Ok(value_to_json(&view.decode(o)?)))
                    .collect::<Res<_>>()?,
            )),
            _ => Ok(json!([])),
        },
        "dependents" => {
            let e = eid_arg(args)?;
            Ok(json!(view
                .dependents(e)?
                .iter()
                .map(|e| e.n())
                .collect::<Vec<_>>()))
        }
        "bundle" => Ok(view.bundle(eid_arg(args)?)?.to_json()),
        other => Err(arg(format!("unknown read operation {other:?}"))),
    }
}

/// The statement id in `eid`: a number or `{"stmt": n}`.
fn eid_arg(args: &J) -> Res<Eid> {
    let j = args
        .get("eid")
        .filter(|j| !j.is_null())
        .ok_or_else(|| arg("`eid` is required"))?;
    eid_from_json(j, &HashMap::new())
}

fn str_arg<'a>(args: &'a J, key: &str) -> Res<&'a str> {
    args.get(key)
        .and_then(J::as_str)
        .ok_or_else(|| arg(format!("`{key}` must be a string")))
}

enum Lookup {
    Absent,
    Missing,
    Found(ObjectId),
}

impl Lookup {
    fn id(self) -> Option<ObjectId> {
        match self {
            Lookup::Found(i) => Some(i),
            _ => None,
        }
    }
}

fn opt_id(view: &View<'_>, args: &J, key: &str) -> Res<Lookup> {
    match args.get(key) {
        None | Some(J::Null) => Ok(Lookup::Absent),
        Some(j) => Ok(match view.encode(&value_from_json(j)?)? {
            Some(id) => Lookup::Found(id),
            None => Lookup::Missing,
        }),
    }
}

fn required_id(view: &View<'_>, args: &J, key: &str) -> Res<Option<ObjectId>> {
    match args.get(key) {
        None | Some(J::Null) => Err(arg(format!("`{key}` is required"))),
        Some(j) => Ok(view.encode(&value_from_json(j)?)?),
    }
}

fn triple_json(view: &View<'_>, t: &Triple) -> Res<J> {
    let ms = |o: Option<i64>| o.map_or(J::Null, |v| json!(v));
    Ok(json!({
        "eid": t.eid.n(),
        "s": value_to_json(&view.decode(t.s)?),
        "p": value_to_json(&view.decode(t.p)?),
        "o": value_to_json(&view.decode(t.o)?),
        "tAdd": t.t_add.0,
        "tRet": t.t_ret.map_or(J::Null, |x| json!(x.0)),
        "validFrom": ms(t.v_from),
        "validTo": ms(t.v_to),
        "retKind": t.ret_kind.map_or(J::Null, |k| json!(ret_kind(k))),
    }))
}

pub(crate) fn ret_kind(k: tiramemsu::RetKind) -> &'static str {
    match k {
        tiramemsu::RetKind::Explicit => "explicit",
        tiramemsu::RetKind::Cascade => "cascade",
        tiramemsu::RetKind::Supersede => "supersede",
        tiramemsu::RetKind::Cardinality => "cardinality",
    }
}

fn path_row_json(view: &View<'_>, r: &PathRow) -> Res<J> {
    let path = match &r.path {
        None => J::Null,
        Some(p) => json!({
            "nodes": p.nodes.iter().map(|n| Ok(value_to_json(&view.decode(*n)?))).collect::<Res<Vec<_>>>()?,
            "hops": p.hops.iter().map(|h| Ok(json!({
                "eid": value_to_json(&view.decode(h.eid)?),
                "predicate": value_to_json(&view.decode(h.pred)?),
                "dir": if h.dir == PathDir::Out { "out" } else { "in" },
                "kind": format!("{:?}", h.kind).to_lowercase(),
            }))).collect::<Res<Vec<_>>>()?,
        }),
    };
    Ok(json!({
        "start": value_to_json(&view.decode(r.start)?),
        "end": value_to_json(&view.decode(r.end)?),
        "hops": r.hops,
        "path": path,
    }))
}

fn event_json(e: &Event) -> J {
    json!({
        "t": e.t.0,
        "eid": e.eid.n(),
        "op": if e.op == Op::Assert { "assert" } else { "retract" },
        "kind": e.kind.map_or(J::Null, |k| json!(ret_kind(k))),
    })
}

/// A transaction report as JSON.
pub fn report_json(r: &TxReport) -> J {
    let kinds = |v: &[(tiramemsu::Eid, tiramemsu::RetKind)]| {
        v.iter()
            .map(|(e, k)| json!({ "eid": e.n(), "kind": ret_kind(*k) }))
            .collect::<Vec<_>>()
    };
    let ids = |v: &[tiramemsu::Eid]| v.iter().map(|e| e.n()).collect::<Vec<_>>();
    json!({
        "t": r.t.0,
        "instant": r.instant,
        "asserted": ids(&r.asserted),
        "existing": ids(&r.existing),
        "retracted": kinds(&r.retracted),
        "superseded": r.superseded.iter().map(|(o, n)| json!({ "old": o.n(), "new": n.n() })).collect::<Vec<_>>(),
        "memberships": ids(&r.memberships),
        "membershipsRetracted": kinds(&r.memberships_retracted),
    })
}

fn sparql_json(r: &SparqlResult) -> J {
    match r {
        SparqlResult::Solutions(s) => {
            let rows = s.rows.iter().map(|row| {
                let mut o = Map::new();
                for (var, cell) in s.vars.iter().zip(row) {
                    if let Some(v) = cell {
                        o.insert(var.clone(), value_to_json(v));
                    }
                }
                J::Object(o)
            });
            json!({ "kind": "select", "vars": s.vars, "rows": rows.collect::<Vec<_>>() })
        }
        SparqlResult::Boolean(b) => json!({ "kind": "ask", "value": b }),
        SparqlResult::Graph(g) => json!({
            "kind": "graph",
            "triples": g.iter().map(rdf_triple_json).collect::<Vec<_>>(),
        }),
        SparqlResult::Update(r) => json!({ "kind": "update", "report": report_json(r) }),
    }
}

fn rdf_triple_json(t: &RdfTriple) -> J {
    json!({ "s": rdf_term_json(&t.s), "p": rdf_term_json(&t.p), "o": rdf_term_json(&t.o) })
}

fn rdf_term_json(t: &RdfTerm) -> J {
    match t {
        RdfTerm::Iri(i) => json!({ "iri": i }),
        RdfTerm::Blank(b) => json!({ "bnodeLabel": b }),
        RdfTerm::Literal {
            lex,
            datatype: None,
            lang: None,
        } => json!(lex),
        RdfTerm::Literal {
            lex, lang: Some(l), ..
        } => json!({ "lex": lex, "lang": l }),
        RdfTerm::Literal {
            lex,
            datatype: Some(d),
            ..
        } => json!({ "lex": lex, "datatype": d }),
        RdfTerm::Triple(t) => json!({ "triple": rdf_triple_json(t) }),
    }
}
