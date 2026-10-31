//! Write operations: transactions as lists of op objects, Cypher writes and speculation.

use serde_json::{json, Value as J};
use std::collections::HashMap;
use tiramemsu::{
    AssertOpts, Asserted, BulkImport, Bundle, BundleFormat, Db, Eid, Error, ImportProgress,
    ObjectId, OnExisting, Patch, PatchField, QueryBudget, Tx, TxCypher, TxOptions, TxReport, Valid,
};

use crate::read::{report_json, run};
use crate::value::{eid_from_json, into_core, params_from_json, time_from_json, value_from_json};
use crate::{arg, tx_options, BindError, Res};

/// `{"ops": [...], "options": {...}}` as one transaction; returns the report with
/// `results` (one per op) and `refs` (the eids named with `as`). With a budget the
/// transaction is bounded by it and a stopped one commits nothing.
pub fn transact(db: &Db, args: &J, budget: Option<&QueryBudget>) -> Res<J> {
    run_ops(args, |options, body| match budget {
        Some(b) => db.transact_budgeted(options, b, body),
        None => db.transact(options, body),
    })
}

/// `{"session", "ops", "options"}` as one chunk of a bulk import session: the
/// same op list and result as `transact`, committed under the session's lease.
// @lat: [[bindings#JSON Bridge#Bulk Import]]
pub fn import_chunk(
    session: &mut BulkImport<'_>,
    args: &J,
    budget: Option<&QueryBudget>,
) -> Res<J> {
    run_ops(args, |options, body| {
        session.chunk_with(options, budget, body)
    })
}

/// Applies the `ops` of `args` as the body handed to `commit`, and returns the
/// report with `results` and `refs`.
fn run_ops(
    args: &J,
    commit: impl FnOnce(
        TxOptions,
        &mut dyn FnMut(&mut Tx<'_>) -> Result<(), Error>,
    ) -> Result<TxReport, Error>,
) -> Res<J> {
    let ops = ops_arg(args)?;
    let options = tx_options(args.get("options").unwrap_or(&J::Null))?;
    let mut results = Vec::new();
    let mut refs = HashMap::new();
    let mut body = |tx: &mut Tx<'_>| {
        let (r, named) = apply(tx, ops).map_err(into_core)?;
        results = r;
        refs = named;
        Ok(())
    };
    let report = commit(options, &mut body)?;
    let mut out = report_json(&report);
    out["results"] = J::Array(results);
    out["refs"] = J::Object(refs.into_iter().map(|(k, e)| (k, json!(e.n()))).collect());
    Ok(out)
}

/// The progress of a bulk import session as JSON.
pub fn progress_json(p: &ImportProgress) -> J {
    json!({
        "chunks": p.chunks,
        "rejected": p.rejected,
        "asserted": p.asserted,
        "existing": p.existing,
        "retracted": p.retracted,
        "txs": p.txs.iter().map(|t| t.0).collect::<Vec<_>>(),
        "elapsedMs": p.elapsed.as_secs_f64() * 1000.0,
        "maintenanceMs": p.maintenance.as_secs_f64() * 1000.0,
    })
}

/// `{"text", "params", "options"}`: one Cypher query that may write, in one transaction.
pub fn cypher_write(db: &Db, args: &J, budget: Option<&QueryBudget>) -> Res<J> {
    let text = args
        .get("text")
        .and_then(J::as_str)
        .ok_or_else(|| arg("`text` must be a string"))?;
    let params = params_from_json(args.get("params").unwrap_or(&J::Null))?;
    let options = tx_options(args.get("options").unwrap_or(&J::Null))?;
    let r = match budget {
        Some(b) => db.cypher_write_budgeted(options, text, &params, b)?,
        None => db.cypher_write(options, text, &params)?,
    };
    let mut out = r.to_json();
    out["report"] = r.report.as_ref().map_or(J::Null, report_json);
    Ok(out)
}

/// `{"ops": [...], "queries": [{"op": "sparql", ...}, ...]}`: applies `ops`
/// hypothetically, runs each query on the result, and keeps nothing.
pub fn with(db: &Db, args: &J) -> Res<J> {
    let ops = ops_arg(args)?;
    let queries = args
        .get("queries")
        .and_then(J::as_array)
        .ok_or_else(|| arg("`queries` must be a list"))?;
    let results = db.with(
        |tx| apply(tx, ops).map(|_| ()).map_err(into_core),
        |view| {
            queries
                .iter()
                .map(|q| {
                    let op = q
                        .get("op")
                        .and_then(J::as_str)
                        .ok_or_else(|| arg("a query needs an `op`"))?;
                    run(view, op, q)
                })
                .collect::<Res<Vec<_>>>()
                .map_err(into_core)
        },
    )?;
    Ok(json!({ "results": results }))
}

fn ops_arg(args: &J) -> Res<&Vec<J>> {
    args.get("ops")
        .and_then(J::as_array)
        .ok_or_else(|| arg("`ops` must be a list"))
}

fn valid(op: &J) -> Res<Valid> {
    let bound = |k: &str| op.get(k).map_or(Ok(None), time_from_json);
    Ok(Valid {
        from: bound("validFrom")?,
        to: bound("validTo")?,
    })
}

/// A term, where `{"ref": name}` stands for the statement an earlier op named with `as`.
/// The names bound by `as` in one op list: statements (`assert`, `create`, …) and
/// nodes (`newNode`). Derefs to the statement map for eid positions.
#[derive(Default)]
struct Refs {
    stmts: HashMap<String, Eid>,
    nodes: HashMap<String, tiramemsu::Value>,
}

impl std::ops::Deref for Refs {
    type Target = HashMap<String, Eid>;
    fn deref(&self) -> &Self::Target {
        &self.stmts
    }
}

fn resolve(j: &J, refs: &Refs) -> Res<tiramemsu::Value> {
    if let Some(name) = j.get("ref").and_then(J::as_str) {
        if let Some(node) = refs.nodes.get(name) {
            return Ok(node.clone());
        }
    }
    if j.get("ref").is_some() {
        return Ok(tiramemsu::Value::Stmt(eid_from_json(j, refs)?));
    }
    value_from_json(j)
}

fn term(tx: &mut Tx<'_>, op: &J, key: &str, refs: &Refs) -> Res<ObjectId> {
    let j = op
        .get(key)
        .ok_or_else(|| arg(format!("`{key}` is required")))?;
    Ok(tx.encode(resolve(j, refs)?)?)
}

/// The `model` of an embedding op: an IRI term, or a plain string read as an IRI.
fn model_term(tx: &mut Tx<'_>, op: &J, refs: &Refs) -> Res<ObjectId> {
    match op.get("model") {
        Some(J::String(s)) => Ok(tx.encode(tiramemsu::Value::iri(s.clone()))?),
        _ => term(tx, op, "model", refs),
    }
}

/// A pattern position: absent means "any", a term that is not stored matches nothing.
fn pattern(tx: &mut Tx<'_>, op: &J, key: &str, refs: &Refs) -> Res<Option<Option<ObjectId>>> {
    match op.get(key) {
        None | Some(J::Null) => Ok(Some(None)),
        Some(j) => Ok(tx.lookup(&resolve(j, refs)?)?.map(Some)),
    }
}

fn eid(op: &J, key: &str, refs: &HashMap<String, Eid>) -> Res<Eid> {
    eid_from_json(
        op.get(key)
            .ok_or_else(|| arg(format!("`{key}` is required")))?,
        refs,
    )
}

fn assert_opts(op: &J) -> Res<AssertOpts> {
    let on_existing = match op.get("onExisting").and_then(J::as_str).unwrap_or("return") {
        "return" => OnExisting::Return,
        "confirm" => OnExisting::Confirm,
        other => return Err(arg(format!("unknown onExisting {other:?}"))),
    };
    Ok(AssertOpts {
        valid: valid(op)?,
        on_existing,
    })
}

fn patch(j: &J) -> Res<Patch> {
    let obj = j
        .as_object()
        .ok_or_else(|| arg("`patch` must be an object"))?;
    let mut fields = Vec::new();
    for (k, v) in obj {
        let name = match k.as_str() {
            "o" => "o",
            "validFrom" => "v_from",
            "validTo" => "v_to",
            other => other, // rejected by `Patch::from_fields` with InvalidPatch
        };
        let field = if name == "o" {
            PatchField::Value(value_from_json(v)?)
        } else if name == "v_from" || name == "v_to" {
            PatchField::Time(time_from_json(v)?)
        } else {
            PatchField::Value(value_from_json(v)?)
        };
        fields.push((name, field));
    }
    Ok(Patch::from_fields(fields)?)
}

/// Applies the ops in order; returns one result per op and the eids named with `as`.
fn apply(tx: &mut Tx<'_>, ops: &[J]) -> Res<(Vec<J>, HashMap<String, Eid>)> {
    let mut refs = Refs::default();
    let mut results = Vec::with_capacity(ops.len());
    for op in ops {
        let name = op
            .get("op")
            .and_then(J::as_str)
            .ok_or_else(|| arg("an operation needs an `op`"))?;
        let mut named: Option<Eid> = None;
        let result = match name {
            "assert" => {
                let (s, p, o) = (
                    term(tx, op, "s", &refs)?,
                    term(tx, op, "p", &refs)?,
                    term(tx, op, "o", &refs)?,
                );
                let a = tx.assert_with(s, p, o, assert_opts(op)?)?;
                named = Some(a.eid());
                json!({ "eid": a.eid().n(), "new": matches!(a, Asserted::New(_)) })
            }
            "create" => {
                let (s, p, o) = (
                    term(tx, op, "s", &refs)?,
                    term(tx, op, "p", &refs)?,
                    term(tx, op, "o", &refs)?,
                );
                let e = tx.create(s, p, o, valid(op)?)?;
                named = Some(e);
                json!({ "eid": e.n(), "new": true })
            }
            // @tg: implements:: [[openspec:derived-beliefs#Derive on every surface]] {since: "2026-10-28"}
            "derive" => {
                let (s, p, o) = (
                    term(tx, op, "s", &refs)?,
                    term(tx, op, "p", &refs)?,
                    term(tx, op, "o", &refs)?,
                );
                let support = op
                    .get("support")
                    .and_then(J::as_array)
                    .ok_or_else(|| arg("`support` must be a list of statements"))?
                    .iter()
                    .map(|j| eid_from_json(j, &refs))
                    .collect::<Res<Vec<Eid>>>()?;
                let a = tx.derive(s, p, o, valid(op)?, &support)?;
                named = Some(a.eid());
                json!({ "eid": a.eid().n(), "new": matches!(a, Asserted::New(_)) })
            }
            #[cfg(feature = "n3")]
            "storeRules" => {
                let e = crate::n3::store_rules(tx, op)?;
                named = Some(e);
                json!({ "eid": e.n() })
            }
            #[cfg(not(feature = "n3"))]
            "storeRules" => return Err(crate::no_n3()),
            #[cfg(feature = "datalog")]
            "storeProgram" => {
                let e = crate::datalog::store_program(tx, op)?;
                named = Some(e);
                json!({ "eid": e.n() })
            }
            #[cfg(not(feature = "datalog"))]
            "storeProgram" => return Err(crate::no_datalog()),
            #[cfg(feature = "datalog")]
            "enforce" => {
                let e = crate::datalog::enforce(tx, op)?;
                named = e;
                json!({ "eid": e.map(|e| e.n()) })
            }
            #[cfg(not(feature = "datalog"))]
            "enforce" => return Err(crate::no_datalog()),
            #[cfg(feature = "elpi")]
            "storeElpiProgram" => {
                let e = crate::elpi::store_program(tx, op)?;
                named = Some(e);
                json!({ "eid": e.n() })
            }
            #[cfg(feature = "elpi")]
            "enforceElpi" => {
                let e = crate::elpi::enforce(tx, op)?;
                named = e;
                json!({ "eid": e.map(|e| e.n()) })
            }
            #[cfg(not(feature = "elpi"))]
            "storeElpiProgram" | "enforceElpi" => return Err(crate::no_elpi()),
            "retract" => json!(tx.retract(eid(op, "eid", &refs)?)?),
            "retractMatching" => {
                let (s, p, o) = (
                    pattern(tx, op, "s", &refs)?,
                    pattern(tx, op, "p", &refs)?,
                    pattern(tx, op, "o", &refs)?,
                );
                match (s, p, o) {
                    (Some(s), Some(p), Some(o)) => {
                        json!(tx
                            .retract_matching(s, p, o)?
                            .iter()
                            .map(|e| e.n())
                            .collect::<Vec<_>>())
                    }
                    _ => json!([]),
                }
            }
            "supersede" => {
                let root = eid(op, "eid", &refs)?;
                let p = patch(op.get("patch").ok_or_else(|| arg("`patch` is required"))?)?;
                let e = tx.supersede(root, p)?;
                named = Some(e);
                json!({ "eid": e.n() })
            }
            "confirm" => {
                let e = tx.confirm(eid(op, "eid", &refs)?)?;
                json!({ "eid": e.n() })
            }
            "meta" => {
                let (p, o) = (term(tx, op, "p", &refs)?, term(tx, op, "o", &refs)?);
                json!({ "eid": tx.meta(p, o)?.n() })
            }
            "upsert" => {
                let (p, o) = (term(tx, op, "p", &refs)?, term(tx, op, "o", &refs)?);
                let id = tx.upsert(p, o)?;
                json!(value_to_json_of(tx, id)?)
            }
            "newNode" => {
                let id = tx.new_node()?;
                let node = tx.decode(id)?;
                if let Some(label) = op.get("as").and_then(J::as_str) {
                    refs.nodes.insert(label.to_string(), node.clone());
                }
                json!(crate::value::value_to_json(&node))
            }
            "defineEmbeddingModel" => {
                let m = model_term(tx, op, &refs)?;
                let dims = op
                    .get("dimensions")
                    .and_then(J::as_u64)
                    .and_then(|d| u32::try_from(d).ok())
                    .ok_or_else(|| arg("`dimensions` must be a positive integer"))?;
                let metric = match op.get("metric").and_then(J::as_str) {
                    None => tiramemsu::Metric::Cosine,
                    Some(n) => tiramemsu::Metric::from_name(n)
                        .ok_or_else(|| arg("metric must be \"cosine\", \"dot\" or \"l2\""))?,
                };
                let encoding = match op.get("encoding").and_then(J::as_str) {
                    None => tiramemsu::VectorEncoding::F32,
                    Some(n) => tiramemsu::VectorEncoding::from_name(n)
                        .ok_or_else(|| arg("encoding must be \"f32\" or \"f16\""))?,
                };
                let def = tiramemsu::EmbeddingModel {
                    dimensions: dims,
                    metric,
                    encoding,
                };
                json!({ "new": tx.define_embedding_model(m, &def)? })
            }
            "setEmbedding" => {
                let e = eid(op, "eid", &refs)?;
                let m = model_term(tx, op, &refs)?;
                let v = crate::read::vector_arg(op.get("vector"), "vector")?;
                json!({ "new": tx.set_embedding(e, m, &v)? })
            }
            "addToGraph" => {
                let e = eid(op, "eid", &refs)?;
                let g = term(tx, op, "graph", &refs)?;
                let (m, added) = tx.add_to_graph(e, g, assert_opts(op)?)?;
                json!({ "eid": m.n(), "new": added })
            }
            "removeFromGraph" => {
                let e = eid(op, "eid", &refs)?;
                let g = term(tx, op, "graph", &refs)?;
                json!(tx.remove_from_graph(e, g)?)
            }
            "clearGraph" => {
                let g = term(tx, op, "graph", &refs)?;
                json!(tx.clear_graph(g)?.iter().map(|e| e.n()).collect::<Vec<_>>())
            }
            "createGraph" => {
                let g = term(tx, op, "graph", &refs)?;
                json!({ "eid": tx.create_graph(g)?.eid().n() })
            }
            "dropGraph" => {
                let g = term(tx, op, "graph", &refs)?;
                json!(tx.drop_graph(g)?.iter().map(|e| e.n()).collect::<Vec<_>>())
            }
            "importBundle" => {
                let j = op
                    .get("bundle")
                    .ok_or_else(|| arg("`bundle` is required"))?;
                let r = tx.import_bundle(&Bundle::from_json(j)?)?;
                named = Some(r.root);
                json!({
                    "root": r.root.n(),
                    "statements": r.statements.iter().map(|s| json!({
                        "id": s.local, "eid": s.eid.n(), "new": s.new,
                    })).collect::<Vec<_>>(),
                })
            }
            "cypher" => {
                let text = op
                    .get("text")
                    .and_then(J::as_str)
                    .ok_or_else(|| arg("`text` must be a string"))?;
                let params = params_from_json(op.get("params").unwrap_or(&J::Null))?;
                TxCypher::cypher(tx, text, &params)?.to_json()
            }
            other => return Err(arg(format!("unknown transaction operation {other:?}"))),
        };
        if let (Some(label), Some(e)) = (op.get("as").and_then(J::as_str), named) {
            refs.stmts.insert(label.to_string(), e);
        }
        results.push(result);
    }
    Ok((results, refs.stmts))
}

fn value_to_json_of(tx: &mut Tx<'_>, id: ObjectId) -> Result<J, Error> {
    Ok(crate::value::value_to_json(&tx.decode(id)?))
}

impl From<BindError> for Error {
    fn from(e: BindError) -> Error {
        into_core(e)
    }
}

/// `previewBundle`: `{bundle}` to `{wouldCommit, failure, root, statements:
/// [{id, eid, new}], report, burned: {statements, nodes, blankNodes, terms},
/// scope: {basis, t, instant, reserved: false}}`. `failure` is the schema error
/// as `{code, message}` (then `root`, `statements` and `report` are `null`); a
/// malformed bundle fails the call with `InvalidTerm`, a cyclic one with
/// `Unsupported`.
// @lat: [[bindings#JSON Bridge#Conflict Review]]
pub fn preview_bundle(db: &Db, args: &J) -> Res<J> {
    let j = args
        .get("bundle")
        .filter(|j| !j.is_null())
        .ok_or_else(|| arg("`bundle` is required"))?;
    let p = db.preview_bundle(&Bundle::from_json(j)?)?;
    let would = p.would_commit();
    let (root, statements) = match &p.import {
        Some(r) => (
            json!(r.root.n()),
            json!(r
                .statements
                .iter()
                .map(|s| json!({ "id": s.local, "eid": s.eid.n(), "new": s.new }))
                .collect::<Vec<_>>()),
        ),
        None => (J::Null, J::Null),
    };
    Ok(json!({
        "wouldCommit": would,
        "failure": p.failure.map_or(J::Null, |e| BindError::Db(e).to_json()),
        "root": root,
        "statements": statements,
        "report": p.report.as_ref().map_or(J::Null, report_json),
        "burned": {
            "statements": p.burned.statements.iter().map(|e| e.n()).collect::<Vec<_>>(),
            "nodes": p.burned.nodes,
            "blankNodes": p.burned.blank_nodes,
            "terms": p.burned.terms,
        },
        "scope": {
            "basis": p.scope.basis.0,
            "t": p.scope.t.0,
            "instant": p.scope.instant,
            "reserved": false,
        },
    }))
}
