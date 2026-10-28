//! Read operations: views, SPARQL, Cypher, lookups, paths and the event log.

use std::collections::HashMap;

use serde_json::{json, Map, Value as J};
use tiramemsu::{
    BundleFormat, ConflictQuery, Db, Eid, Event, Explain, ObjectId, Op, Params, PathArgs,
    PathCompleteness, PathDir, PathMode, PathRow, RdfTerm, RdfTriple, RecallQuery, RegionKind,
    RouteNote, SparqlOptions, SparqlResult, TextEvidence, TextMode, TextQuery, TimeRef,
    TimeRespecting, Triple, TxReport, VectorQuery, View,
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
            let flag = |key: &str| match args.get(key) {
                None | Some(J::Null) => Ok(false),
                Some(J::Bool(b)) => Ok(*b),
                Some(_) => Err(arg(format!("{key} must be a boolean"))),
            };
            let opts = SparqlOptions {
                provenance: flag("provenance")?,
                query_only: flag("queryOnly")?,
                params: sparql_params(args.get("params").unwrap_or(&J::Null))?,
            };
            let r = view.sparql_with(text, &opts)?;
            let mut out = sparql_json(&r);
            if flag("pathCompleteness")? {
                if let Some(s) = r.solutions() {
                    out["pathCompleteness"] = completeness_json(s.path_completeness);
                }
            }
            Ok(out)
        }
        "cypher" => {
            let text = str_arg(args, "text")?;
            let params = params_from_json(args.get("params").unwrap_or(&J::Null))?;
            let r = view.cypher(text, &params)?;
            let mut out = r.to_json();
            if bool_arg(args, "pathCompleteness")? {
                out["pathCompleteness"] = completeness_json(r.path_completeness);
            }
            Ok(out)
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
            let graphs = match args.get("graphs") {
                None | Some(J::Null) => None,
                Some(J::Array(gs)) => {
                    let mut ids = Vec::with_capacity(gs.len());
                    for g in gs {
                        // a term that is not stored names no graph
                        if let Some(id) = view.encode(&value_from_json(g)?)? {
                            ids.push(id);
                        }
                    }
                    Some(ids)
                }
                Some(_) => return Err(arg("`graphs` must be a list of terms")),
            };
            let time_respecting = match args.get("timeRespecting") {
                None | Some(J::Null) | Some(J::Bool(false)) => None,
                Some(J::Bool(true)) => Some(TimeRespecting::default()),
                Some(J::Object(o)) => {
                    if let Some(k) = o.keys().find(|k| *k != "after") {
                        return Err(arg(format!("unknown timeRespecting option {k:?}")));
                    }
                    Some(TimeRespecting {
                        after: crate::value::time_from_json(o.get("after").unwrap_or(&J::Null))?,
                    })
                }
                Some(_) => {
                    return Err(arg(
                        "`timeRespecting` must be a boolean or {\"after\": time}",
                    ))
                }
            };
            let opts = PathArgs {
                mode,
                max_hops: max,
                graphs,
                time_respecting,
                capped: bool_arg(args, "capped")?,
            };
            let report = view.path_report(start, str_arg(args, "path")?, &opts)?;
            let rows = J::Array(
                report
                    .rows
                    .iter()
                    .map(|r| path_row_json(view, r))
                    .collect::<Res<_>>()?,
            );
            // the plain array unless the completeness is asked for
            Ok(if bool_arg(args, "completeness")? {
                json!({
                    "rows": rows,
                    "completeness": completeness_json(Some(report.completeness)),
                })
            } else {
                rows
            })
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
        "textSearch" => text_search(view, args),
        "vectorSearch" => vector_search(view, args),
        "recall" => recall(view, args),
        "unembedded" => {
            let model = model_arg(view, args)?;
            let limit = match args.get("limit").filter(|v| !v.is_null()) {
                None => None,
                Some(l) => Some(
                    l.as_u64()
                        .ok_or_else(|| arg("limit must be a non-negative integer"))?
                        as usize,
                ),
            };
            Ok(json!(view
                .unembedded(model, limit)?
                .iter()
                .map(|e| e.n())
                .collect::<Vec<_>>()))
        }
        "conflicts" => conflicts(view, args),
        "explainSparql" => Ok(explain_json(&view.explain_sparql(str_arg(args, "text")?)?)),
        #[cfg(feature = "n3")]
        "reason" => crate::n3::reason(view, args),
        #[cfg(feature = "n3")]
        "ruleSetInfo" => crate::n3::rule_set_info(view, args),
        #[cfg(not(feature = "n3"))]
        "reason" | "ruleSetInfo" => Err(crate::no_n3()),
        #[cfg(feature = "datalog")]
        "datalog" => crate::datalog::query(view, args),
        #[cfg(feature = "datalog")]
        "datalogReason" => crate::datalog::reason(view, args),
        #[cfg(feature = "datalog")]
        "programInfo" => crate::datalog::program_info(view, args),
        #[cfg(feature = "datalog")]
        "explainDatalog" => crate::datalog::explain(view, args),
        #[cfg(feature = "datalog")]
        "check" => crate::datalog::check(view, args),
        #[cfg(not(feature = "datalog"))]
        "datalog" | "datalogReason" | "programInfo" | "explainDatalog" | "check" => {
            Err(crate::no_datalog())
        }
        other => Err(arg(format!("unknown read operation {other:?}"))),
    }
}

/// An explained plan: `{regions: [{kind, note, aliases, queryPlan}], sql,
/// shortCircuit, queryPlan}` with camelCase kinds (`sql`, `nativePath`,
/// `nativeLftj`, `native` for the whole native runner on redb) and route notes (`none`, `cyclicLftjDisabled`, `lftjUnavailable`,
/// `lftjUnsupportedShape`, `lftjBelowEstimate`, `lftjNative`, `pathForward`,
/// `pathInverted`). Parameters are not included.
// @lat: [[bindings#JSON Bridge#Explain]]
pub fn explain_json(ex: &Explain) -> J {
    let kind = |k: RegionKind| match k {
        RegionKind::Sql => "sql",
        RegionKind::NativePath => "nativePath",
        RegionKind::NativeLftj => "nativeLftj",
        RegionKind::Native => "native",
    };
    let note = |n: RouteNote| match n {
        RouteNote::None => "none",
        RouteNote::CyclicLftjDisabled => "cyclicLftjDisabled",
        RouteNote::LftjUnavailable => "lftjUnavailable",
        RouteNote::LftjUnsupportedShape => "lftjUnsupportedShape",
        RouteNote::LftjBelowEstimate => "lftjBelowEstimate",
        RouteNote::LftjNative => "lftjNative",
        RouteNote::PathForward => "pathForward",
        RouteNote::PathInverted => "pathInverted",
    };
    json!({
        "regions": ex.regions.iter().map(|r| json!({
            "kind": kind(r.kind),
            "note": note(r.note),
            "aliases": r.aliases,
            "queryPlan": r.query_plan,
        })).collect::<Vec<_>>(),
        "sql": ex.sql,
        "shortCircuit": ex.short_circuit,
        "queryPlan": ex.query_plan,
    })
}

/// `textSearch`: `{text, mode?, graphs?, predicates?, limit?, confidence?}` to the
/// ranked hits, each with its statement, scores and evidence (absent confidence is
/// `null`).
// @lat: [[bindings#JSON Bridge#Text Recall]]
fn text_search(view: &View<'_>, args: &J) -> Res<J> {
    let mut q = TextQuery::new(str_arg(args, "text")?);
    for (k, v) in args.as_object().into_iter().flatten() {
        if v.is_null() {
            continue;
        }
        match k.as_str() {
            "text" | "view" | "budget" => {}
            "mode" => {
                q.mode = v
                    .as_str()
                    .and_then(TextMode::from_name)
                    .ok_or_else(|| arg("mode must be \"all\", \"any\" or \"phrase\""))?
            }
            "limit" => {
                q.limit = Some(
                    v.as_u64()
                        .ok_or_else(|| arg("limit must be a non-negative integer"))?
                        as usize,
                )
            }
            "graphs" | "predicates" => {
                let J::Array(items) = v else {
                    return Err(arg(format!("`{k}` must be a list of terms")));
                };
                let mut ids = Vec::with_capacity(items.len());
                for t in items {
                    // a term that is not stored matches nothing
                    if let Some(id) = view.encode(&value_from_json(t)?)? {
                        ids.push(id);
                    }
                }
                if k == "graphs" {
                    q.graphs = Some(ids);
                } else {
                    q.predicates = Some(ids);
                }
            }
            "confidence" => match view.encode(&value_from_json(v)?)? {
                Some(id) => q.confidence = Some(id),
                // a predicate that is not stored is on no statement
                None => q.confidence = Some(ObjectId::from_raw(0)),
            },
            other => return Err(arg(format!("unknown textSearch option {other:?}"))),
        }
    }
    let hits = view.text_search(&q)?;
    Ok(J::Array(
        hits.iter()
            .map(|h| {
                Ok(json!({
                    "eid": h.eid.n(),
                    "s": value_to_json(&view.decode(h.s)?),
                    "p": value_to_json(&view.decode(h.p)?),
                    "o": value_to_json(&view.decode(h.o)?),
                    "text": h.text,
                    "lang": h.lang,
                    "score": h.lexical,
                    "rank": h.rank,
                    "evidence": {
                        "confidence": h.evidence.confidence,
                        "confirmations": h.evidence.confirmations,
                        "authors": h.evidence.authors,
                        "tAdd": h.evidence.t_add.0,
                        "addedAt": h.evidence.added_at,
                    },
                }))
            })
            .collect::<Res<_>>()?,
    ))
}

fn evidence_json(e: &TextEvidence) -> J {
    json!({
        "confidence": e.confidence,
        "confirmations": e.confirmations,
        "authors": e.authors,
        "tAdd": e.t_add.0,
        "addedAt": e.added_at,
    })
}

/// The `model` argument: an IRI term (or string) naming a defined model. A model
/// that was never stored is undefined.
fn model_arg(view: &View<'_>, args: &J) -> Res<ObjectId> {
    let j = args
        .get("model")
        .filter(|v| !v.is_null())
        .ok_or_else(|| arg("`model` is required"))?;
    let v = match j {
        J::String(s) => tiramemsu::Value::iri(s.clone()),
        other => value_from_json(other)?,
    };
    view.encode(&v)?.ok_or_else(|| {
        tiramemsu::Error::invalid_query(format!(
            "{v} is not a defined embedding model (define it with defineEmbeddingModel)"
        ))
        .into()
    })
}

/// A list of numbers as an `f32` vector.
pub(crate) fn vector_arg(j: Option<&J>, key: &str) -> Res<Vec<f32>> {
    let Some(J::Array(items)) = j else {
        return Err(arg(format!("`{key}` must be a list of numbers")));
    };
    items
        .iter()
        .map(|x| {
            x.as_f64()
                .map(|f| f as f32)
                .ok_or_else(|| arg(format!("`{key}` must be a list of numbers")))
        })
        .collect()
}

fn usize_arg(v: &J, k: &str) -> Res<usize> {
    v.as_u64()
        .map(|n| n as usize)
        .ok_or_else(|| arg(format!("{k} must be a non-negative integer")))
}

fn term_list(view: &View<'_>, k: &str, v: &J) -> Res<Vec<ObjectId>> {
    let J::Array(items) = v else {
        return Err(arg(format!("`{k}` must be a list of terms")));
    };
    let mut ids = Vec::with_capacity(items.len());
    for t in items {
        // a term that is not stored matches nothing
        if let Some(id) = view.encode(&value_from_json(t)?)? {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// `vectorSearch`: `{model, vector, limit?, graphs?, predicates?, prefilter?,
/// confidence?}` to `{hits: [{eid, s, p, o, similarity, rank, evidence}],
/// candidates, exact}`.
// @lat: [[bindings#JSON Bridge#Vector Recall]]
fn vector_search(view: &View<'_>, args: &J) -> Res<J> {
    let mut q = VectorQuery::new(
        model_arg(view, args)?,
        vector_arg(args.get("vector"), "vector")?,
    );
    for (k, v) in args.as_object().into_iter().flatten() {
        if v.is_null() {
            continue;
        }
        match k.as_str() {
            "model" | "vector" | "view" | "budget" => {}
            "limit" => q.limit = Some(usize_arg(v, k)?),
            "prefilter" => q.prefilter = Some(usize_arg(v, k)?),
            "graphs" => q.graphs = Some(term_list(view, k, v)?),
            "predicates" => q.predicates = Some(term_list(view, k, v)?),
            "confidence" => {
                q.confidence = Some(
                    view.encode(&value_from_json(v)?)?
                        .unwrap_or(ObjectId::from_raw(0)),
                )
            }
            other => return Err(arg(format!("unknown vectorSearch option {other:?}"))),
        }
    }
    let r = view.vector_search(&q)?;
    Ok(json!({
        "hits": r.hits.iter().map(|h| Ok(json!({
            "eid": h.eid.n(),
            "s": value_to_json(&view.decode(h.s)?),
            "p": value_to_json(&view.decode(h.p)?),
            "o": value_to_json(&view.decode(h.o)?),
            "similarity": h.similarity,
            "rank": h.rank,
            "evidence": evidence_json(&h.evidence),
        }))).collect::<Res<Vec<_>>>()?,
        "candidates": r.candidates,
        "exact": r.exact,
    }))
}

/// `recall`: `{text?, mode?, model?, vector?, limit?, depth?, expand?, rrfK?,
/// graphs?, predicates?, prefilter?}` (text, a model with a vector, or both) to the
/// fused hits `[{eid, s, p, o, score, rank, lexical, similarity, textRank,
/// vectorRank, evidence, via}]`. `graphs` and `predicates` apply to both channels.
// @lat: [[bindings#JSON Bridge#Hybrid Recall]]
fn recall(view: &View<'_>, args: &J) -> Res<J> {
    let get = |k: &str| args.get(k).filter(|v| !v.is_null());
    let mut q = RecallQuery::default();
    let (mut graphs, mut predicates) = (None, None);
    for (k, v) in args.as_object().into_iter().flatten() {
        if v.is_null() {
            continue;
        }
        match k.as_str() {
            "text" | "mode" | "model" | "vector" | "prefilter" | "view" | "budget" => {}
            "limit" => q.limit = Some(usize_arg(v, k)?),
            "depth" => q.depth = Some(usize_arg(v, k)?),
            "expand" => q.expand = usize_arg(v, k)?,
            "rrfK" => {
                q.rrf_k = u32::try_from(usize_arg(v, k)?).map_err(|_| arg("rrfK is too large"))?
            }
            "graphs" => graphs = Some(term_list(view, k, v)?),
            "predicates" => predicates = Some(term_list(view, k, v)?),
            other => return Err(arg(format!("unknown recall option {other:?}"))),
        }
    }
    if let Some(t) = get("text") {
        let mut tq = TextQuery::new(t.as_str().ok_or_else(|| arg("text must be a string"))?);
        if let Some(m) = get("mode") {
            tq.mode = m
                .as_str()
                .and_then(TextMode::from_name)
                .ok_or_else(|| arg("mode must be \"all\", \"any\" or \"phrase\""))?;
        }
        tq.graphs = graphs.clone();
        tq.predicates = predicates.clone();
        q.text = Some(tq);
    }
    if get("vector").is_some() || get("model").is_some() {
        let mut vq = VectorQuery::new(model_arg(view, args)?, vector_arg(get("vector"), "vector")?);
        vq.graphs = graphs;
        vq.predicates = predicates;
        if let Some(n) = get("prefilter") {
            vq.prefilter = Some(usize_arg(n, "prefilter")?);
        }
        q.vector = Some(vq);
    }
    let hits = view.recall(&q)?;
    Ok(J::Array(
        hits.iter()
            .map(|h| {
                Ok(json!({
                    "eid": h.eid.n(),
                    "s": value_to_json(&view.decode(h.s)?),
                    "p": value_to_json(&view.decode(h.p)?),
                    "o": value_to_json(&view.decode(h.o)?),
                    "score": h.score,
                    "rank": h.rank,
                    "lexical": h.lexical,
                    "similarity": h.similarity,
                    "textRank": h.text_rank,
                    "vectorRank": h.vector_rank,
                    "evidence": evidence_json(&h.evidence),
                    "via": h.via.map(|e| e.n()),
                }))
            })
            .collect::<Res<_>>()?,
    ))
}

/// `conflicts`: `{s?, p?, limit?, confidence?, source?}` to the view's
/// conflicts, each `{s, p, declaredMany, overlaps: [{validFrom, validTo}],
/// values: [{o, statements: [{eid, validFrom, validTo, tAdd, addedAt,
/// confidence, confirmedBy, authors, sources, sourceLayer}]}]}`. Absent
/// confidence is `null`; an unbounded bound is `null`. A subject or predicate
/// that is not stored matches nothing.
// @lat: [[bindings#JSON Bridge#Conflict Review]]
fn conflicts(view: &View<'_>, args: &J) -> Res<J> {
    let mut q = ConflictQuery::default();
    for (k, v) in args.as_object().into_iter().flatten() {
        if v.is_null() {
            continue;
        }
        match k.as_str() {
            "view" | "budget" => {}
            "limit" => {
                q.limit = Some(
                    v.as_u64()
                        .ok_or_else(|| arg("limit must be a non-negative integer"))?
                        as usize,
                )
            }
            "s" | "p" | "confidence" | "source" => {
                // a term that is not stored is on no statement
                let id = view
                    .encode(&value_from_json(v)?)?
                    .unwrap_or(ObjectId::from_raw(0));
                match k.as_str() {
                    "s" => q.subject = Some(id),
                    "p" => q.predicate = Some(id),
                    "confidence" => q.confidence = Some(id),
                    _ => q.source = Some(id),
                }
            }
            other => return Err(arg(format!("unknown conflicts option {other:?}"))),
        }
    }
    let ms = |o: Option<i64>| o.map_or(J::Null, |v| json!(v));
    let terms = |ids: &[ObjectId]| -> Res<Vec<J>> {
        ids.iter()
            .map(|i| Ok(value_to_json(&view.decode(*i)?)))
            .collect()
    };
    let mut out = Vec::new();
    for c in view.conflicts(&q)? {
        let mut values = Vec::with_capacity(c.values.len());
        for val in &c.values {
            let mut statements = Vec::with_capacity(val.statements.len());
            for e in &val.statements {
                statements.push(json!({
                    "eid": e.eid.n(),
                    "validFrom": ms(e.valid.from),
                    "validTo": ms(e.valid.to),
                    "tAdd": e.t_add.0,
                    "addedAt": e.added_at,
                    "confidence": e.confidence,
                    "confirmedBy": e.confirmed_by.iter().map(|t| t.0).collect::<Vec<_>>(),
                    "authors": terms(&e.authors)?,
                    "sources": terms(&e.sources)?,
                    "sourceLayer": terms(&e.source_layer)?,
                }));
            }
            values.push(
                json!({ "o": value_to_json(&view.decode(val.o)?), "statements": statements }),
            );
        }
        out.push(json!({
            "s": value_to_json(&view.decode(c.s)?),
            "p": value_to_json(&view.decode(c.p)?),
            "declaredMany": c.declared_many,
            "overlaps": c.overlaps.iter().map(|w| json!({
                "validFrom": ms(w.from),
                "validTo": ms(w.to),
            })).collect::<Vec<_>>(),
            "values": values,
        }));
    }
    Ok(J::Array(out))
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

/// An optional boolean argument (`null` or absent is false).
fn bool_arg(args: &J, key: &str) -> Res<bool> {
    match args.get(key) {
        None | Some(J::Null) => Ok(false),
        Some(J::Bool(b)) => Ok(*b),
        Some(_) => Err(arg(format!("{key} must be a boolean"))),
    }
}

/// The SPARQL execution parameters: `{"name": time}` with a time an integer of
/// epoch milliseconds or an RFC 3339 date or date-time (the start instants of
/// `SERVICE <urn:tiramemsu:tm:timeRespecting/$name>`).
fn sparql_params(j: &J) -> Res<Params> {
    let mut out = Params::new();
    match j {
        J::Null => {}
        J::Object(o) => {
            for (k, v) in o {
                let ms = crate::value::time_from_json(v)?
                    .ok_or_else(|| arg(format!("parameter {k:?} is null")))?;
                out.insert(
                    k.trim_start_matches('$').to_string(),
                    tiramemsu::Value::Int(ms),
                );
            }
        }
        _ => return Err(arg("params must be an object")),
    }
    Ok(out)
}

/// A path completeness verdict: `{"kind": "exhaustive" | "bound" | "cap",
/// "maxHops": n | null, "complete": bool}`; `null` when no path search ran.
pub(crate) fn completeness_json(c: Option<PathCompleteness>) -> J {
    match c {
        None => J::Null,
        Some(c) => json!({
            "kind": c.kind(),
            "maxHops": c.max_hops(),
            "complete": c.is_complete(),
        }),
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
        "arrival": r.arrival.map_or(J::Null, |a| json!(a)),
    }))
}

pub(crate) fn event_json(e: &Event) -> J {
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

pub(crate) fn sparql_json(r: &SparqlResult) -> J {
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
            let mut out =
                json!({ "kind": "select", "vars": s.vars, "rows": rows.collect::<Vec<_>>() });
            if let Some(prov) = &s.provenance {
                // one list of statements per row, parallel to "rows"
                out["provenance"] = prov
                    .iter()
                    .map(|eids| {
                        eids.iter()
                            .map(|e| value_to_json(&tiramemsu::Value::Stmt(*e)))
                            .collect::<Vec<_>>()
                    })
                    .collect();
                // the query parts whose statements are not cited; empty: complete
                out["provenanceGaps"] = s.provenance_gaps.iter().map(|g| g.name()).collect();
            }
            out
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
