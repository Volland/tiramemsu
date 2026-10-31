//! ELPI through the bridge (feature `elpi`): the read operations `elpi`,
//! `elpiCheck`, `elpiConclude` and `elpiProgramInfo`, the transaction ops
//! `storeElpiProgram` and `enforceElpi`, and `materializeElpi`.
//!
//! A call names its program with exactly one of `program` (ELPI text) and
//! `stored` (a program node; a plain string is an IRI). Result terms are bridge
//! terms, plus `{"elpi": "<printed>"}` for a term no value holds.
// @lat: [[bindings#JSON Bridge#ELPI]]

use serde_json::{json, Value as J};
use tiramemsu::{
    Db, Eid, ElpiOptions, ElpiProgram, ElpiProgramInfo, ElpiTerm, MaterializeElpiOptions, Tx,
    TxElpi, Valid, View,
};

use crate::read::report_json;
use crate::value::{time_from_json, value_from_json, value_to_json};
use crate::{arg, Res};

/// A program node: a plain string is an IRI.
fn node_from_json(j: &J) -> Res<tiramemsu::Value> {
    match j {
        J::String(s) => Ok(tiramemsu::Value::iri(s.clone())),
        J::Null => Err(arg("a program node is required")),
        other => value_from_json(other),
    }
}

fn program_arg(args: &J) -> Res<ElpiProgram> {
    let get = |k: &str| args.get(k).filter(|v| !v.is_null());
    match (get("program"), get("stored")) {
        (Some(J::String(text)), None) => Ok(ElpiProgram::Text(text.clone())),
        (Some(_), None) => Err(arg("`program` must be ELPI text")),
        (None, Some(n)) => Ok(ElpiProgram::Stored(node_from_json(n)?)),
        _ => Err(arg(
            "give exactly one of `program` (ELPI text) and `stored`",
        )),
    }
}

fn uint(args: &J, key: &str) -> Res<u64> {
    match args.get(key) {
        None | Some(J::Null) => Ok(0),
        Some(v) => v
            .as_u64()
            .ok_or_else(|| arg(format!("`{key}` must be a non-negative integer"))),
    }
}

/// `validAt`, `fuel` (resolution steps, 0 the engine's default), `limit`
/// (solutions, 0 all) and `maxRows`.
fn options(args: &J) -> Res<ElpiOptions> {
    let mut o = ElpiOptions {
        valid_at: time_from_json(args.get("validAt").unwrap_or(&J::Null))?,
        fuel: uint(args, "fuel")?,
        limit: uint(args, "limit")? as usize,
        ..ElpiOptions::default()
    };
    if let Some(m) = args.get("maxRows").filter(|v| !v.is_null()) {
        o.max_rows = m
            .as_u64()
            .ok_or_else(|| arg("`maxRows` must be a non-negative integer"))?;
    }
    Ok(o)
}

fn term_json(t: &ElpiTerm) -> J {
    match t {
        ElpiTerm::Value(v) => value_to_json(v),
        ElpiTerm::Other(o) => json!({ "elpi": o }),
    }
}

fn bound(b: Option<i64>) -> J {
    b.map_or(J::Null, |v| json!(v))
}

fn eids(e: &[Eid]) -> J {
    json!(e.iter().map(|e| e.n()).collect::<Vec<_>>())
}

fn valid_json(v: &Valid) -> J {
    json!({ "from": bound(v.from), "to": bound(v.to) })
}

/// The read `elpi`: `{rows: [{var: term}], exhausted, fuelUsed, rowsRead}`.
// @tg: implements:: [[openspec:elpi-query-calls#ELPI bridge operations]] {since: "2026-10-31"}
pub fn query(view: &View<'_>, args: &J) -> Res<J> {
    let goal = args
        .get("goal")
        .and_then(J::as_str)
        .ok_or_else(|| arg("`goal` must be a string"))?;
    let r = view.elpi(&program_arg(args)?, goal, &options(args)?)?;
    let rows: Vec<J> = r
        .rows
        .iter()
        .map(|row| {
            J::Object(
                row.bindings
                    .iter()
                    .map(|(k, t)| (k.clone(), term_json(t)))
                    .collect(),
            )
        })
        .collect();
    Ok(json!({
        "rows": rows,
        "exhausted": r.exhausted,
        "fuelUsed": r.cost.fuel_used,
        "rowsRead": r.cost.rows_read,
    }))
}

/// The read `elpiCheck`: `{violations: [{severity, name, message, eids, valid}], fuelUsed, rowsRead}`.
pub fn check(view: &View<'_>, args: &J) -> Res<J> {
    let r = view.elpi_check(&program_arg(args)?, &options(args)?)?;
    let violations: Vec<J> = r
        .violations
        .iter()
        .map(|v| {
            json!({
                "severity": v.severity.name(),
                "name": v.name,
                "message": v.message,
                "eids": eids(&v.eids),
                "valid": valid_json(&v.valid),
            })
        })
        .collect();
    Ok(json!({
        "violations": violations,
        "fuelUsed": r.cost.fuel_used,
        "rowsRead": r.cost.rows_read,
    }))
}

/// The read `elpiConclude`: `{conclusions: [{s, p, o, support, validFrom, validTo}], nonMonotonic}`.
pub fn conclude(view: &View<'_>, args: &J) -> Res<J> {
    let r = view.elpi_conclude(&program_arg(args)?, &options(args)?)?;
    let conclusions: Vec<J> = r
        .conclusions
        .iter()
        .map(|c| {
            json!({
                "s": value_to_json(&c.s),
                "p": value_to_json(&c.p),
                "o": value_to_json(&c.o),
                "support": eids(&c.support),
                "validFrom": bound(c.valid.from),
                "validTo": bound(c.valid.to),
            })
        })
        .collect();
    Ok(json!({
        "conclusions": conclusions,
        "nonMonotonic": r.non_monotonic,
        "fuelUsed": r.cost.fuel_used,
        "rowsRead": r.cost.rows_read,
    }))
}

fn info_json(i: &ElpiProgramInfo) -> J {
    json!({
        "program": value_to_json(&i.program),
        "statement": i.statement.n(),
        "text": i.text,
        "includes": i.includes.iter().map(|(p, e)| json!({
            "program": value_to_json(p),
            "statement": e.n(),
        })).collect::<Vec<_>>(),
        "nonMonotonic": i.non_monotonic,
        "enforced": i.enforced,
    })
}

/// The read `elpiProgramInfo` (`program`, a node): `{program, statement, text,
/// includes, nonMonotonic, enforced}`.
pub fn program_info(view: &View<'_>, args: &J) -> Res<J> {
    let node = node_from_json(
        args.get("stored")
            .or_else(|| args.get("program"))
            .unwrap_or(&J::Null),
    )?;
    Ok(info_json(&view.elpi_program_info(&node)?))
}

/// The transaction op `storeElpiProgram` (`program`, `text`).
pub fn store_program(tx: &mut Tx<'_>, op: &J) -> Res<Eid> {
    let node = node_from_json(op.get("program").unwrap_or(&J::Null))?;
    let text = op
        .get("text")
        .and_then(J::as_str)
        .ok_or_else(|| arg("`text` must be ELPI text"))?;
    Ok(tx.store_elpi_program(node, text)?)
}

/// The transaction op `enforceElpi` (`program`, `on`, default true).
pub fn enforce(tx: &mut Tx<'_>, op: &J) -> Res<Option<Eid>> {
    let node = node_from_json(op.get("program").unwrap_or(&J::Null))?;
    let on = match op.get("on") {
        None | Some(J::Null) => true,
        Some(J::Bool(b)) => *b,
        Some(_) => return Err(arg("`on` must be a boolean")),
    };
    Ok(tx.enforce_elpi(node, on)?)
}

/// `materializeElpi` (`stored` or `program`, a node; `dryRun`, `fuel`,
/// `maxRows`): `{program, programStatement, reasonedAt, commit, derived,
/// existing, widened, reconciled, staleSupport, needsRerunAfterWrites, dryRun}`.
pub fn materialize(db: &Db, args: &J) -> Res<J> {
    let node = node_from_json(
        args.get("stored")
            .or_else(|| args.get("program"))
            .unwrap_or(&J::Null),
    )?;
    let dry_run = match args.get("dryRun") {
        None | Some(J::Null) => false,
        Some(J::Bool(b)) => *b,
        Some(_) => return Err(arg("`dryRun` must be a boolean")),
    };
    let r = db.materialize_elpi(
        &node,
        &MaterializeElpiOptions {
            elpi: options(args)?,
            dry_run,
        },
    )?;
    Ok(json!({
        "program": value_to_json(&r.program),
        "programStatement": r.program_statement.n(),
        "reasonedAt": r.reasoned_at,
        "commit": r.commit.as_ref().map_or(J::Null, report_json),
        "derived": eids(&r.derived),
        "existing": eids(&r.existing),
        "widened": r.widened.iter().map(|(a, b)| json!([a.n(), b.n()])).collect::<Vec<_>>(),
        "reconciled": eids(&r.reconciled),
        "staleSupport": r.stale_support.len(),
        "needsRerunAfterWrites": r.needs_rerun_after_writes,
        "dryRun": r.dry_run,
    }))
}
