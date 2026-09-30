//! Write operations: transactions as lists of op objects, Cypher writes and speculation.

use serde_json::{json, Value as J};
use std::collections::HashMap;
use tiramemsu::{
    AssertOpts, Asserted, Bundle, BundleFormat, Db, Eid, Error, ObjectId, OnExisting, Patch,
    PatchField, Tx, TxCypher, Valid,
};

use crate::read::{report_json, run};
use crate::value::{eid_from_json, into_core, params_from_json, time_from_json, value_from_json};
use crate::{arg, tx_options, BindError, Res};

/// `{"ops": [...], "options": {...}}` as one transaction; returns the report with
/// `results` (one per op) and `refs` (the eids named with `as`).
pub fn transact(db: &Db, args: &J) -> Res<J> {
    let ops = ops_arg(args)?;
    let options = tx_options(args.get("options").unwrap_or(&J::Null))?;
    let mut results = Vec::new();
    let mut refs = HashMap::new();
    let report = db.transact(options, |tx| {
        let (r, named) = apply(tx, ops).map_err(into_core)?;
        results = r;
        refs = named;
        Ok(())
    })?;
    let mut out = report_json(&report);
    out["results"] = J::Array(results);
    out["refs"] = J::Object(refs.into_iter().map(|(k, e)| (k, json!(e.n()))).collect());
    Ok(out)
}

/// `{"text", "params", "options"}`: one Cypher query that may write, in one transaction.
pub fn cypher_write(db: &Db, args: &J) -> Res<J> {
    let text = args
        .get("text")
        .and_then(J::as_str)
        .ok_or_else(|| arg("`text` must be a string"))?;
    let params = params_from_json(args.get("params").unwrap_or(&J::Null))?;
    let options = tx_options(args.get("options").unwrap_or(&J::Null))?;
    let r = db.cypher_write(options, text, &params)?;
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
fn resolve(j: &J, refs: &HashMap<String, Eid>) -> Res<tiramemsu::Value> {
    if j.get("ref").is_some() {
        return Ok(tiramemsu::Value::Stmt(eid_from_json(j, refs)?));
    }
    value_from_json(j)
}

fn term(tx: &mut Tx<'_>, op: &J, key: &str, refs: &HashMap<String, Eid>) -> Res<ObjectId> {
    let j = op
        .get(key)
        .ok_or_else(|| arg(format!("`{key}` is required")))?;
    Ok(tx.encode(resolve(j, refs)?)?)
}

/// A pattern position: absent means "any", a term that is not stored matches nothing.
fn pattern(
    tx: &mut Tx<'_>,
    op: &J,
    key: &str,
    refs: &HashMap<String, Eid>,
) -> Res<Option<Option<ObjectId>>> {
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
    let mut refs: HashMap<String, Eid> = HashMap::new();
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
                json!(value_to_json_of(tx, id)?)
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
            refs.insert(label.to_string(), e);
        }
        results.push(result);
    }
    Ok((results, refs))
}

fn value_to_json_of(tx: &mut Tx<'_>, id: ObjectId) -> Result<J, Error> {
    Ok(crate::value::value_to_json(&tx.decode(id)?))
}

impl From<BindError> for Error {
    fn from(e: BindError) -> Error {
        into_core(e)
    }
}
