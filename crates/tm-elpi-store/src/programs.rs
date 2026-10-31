//! Program nodes: `elpi:source` statements, includes and program information
//! (`lat.md/elpi#Program Nodes With Includes`).

use tm_core::store::ReadStore;
use tm_core::vocab::{DL_SOURCE, ELPI_ENFORCED, ELPI_SOURCE, N3_SOURCE};
use tm_core::{Eid, Error, ObjectId, Patch, Result, TxHandle as Tx, Valid, Value, ViewSpec};
use tm_elpi::{scan, Part, Source};
use tm_store::dict::{self, TermCache};

use crate::on_tx;

/// The live `elpi:source` of `node` in the view: its text and statement.
pub fn stored(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    node: &Value,
) -> Result<Option<(String, Eid)>> {
    live_object(store, spec, node, ELPI_SOURCE)
}

fn live_object(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    node: &Value,
    p: &str,
) -> Result<Option<(String, Eid)>> {
    let (Some(s), Some(p)) = (
        dict::encode(store, node)?,
        dict::encode(store, &Value::iri(p))?,
    ) else {
        return Ok(None);
    };
    let Some(row) = tm_store::read::triples(store, spec, Some(s), Some(p), None)?.pop() else {
        return Ok(None);
    };
    match TermCache::new(4).decode(store, row.o, false)? {
        Value::Str(text) => Ok(Some((text, row.eid))),
        _ => Ok(None),
    }
}

fn walk(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    node: &Value,
    text: &str,
    source: Option<Eid>,
    stack: &mut Vec<Value>,
    out: &mut Source,
) -> Result<()> {
    stack.push(node.clone());
    for inc in scan::includes(text) {
        let v = Value::iri(inc);
        if let Some(i) = stack.iter().position(|n| *n == v) {
            let mut chain: Vec<String> = stack[i..].iter().map(Value::to_string).collect();
            chain.push(v.to_string());
            return Err(Error::ProgramCycle {
                program: stack[0].to_string(),
                chain,
            });
        }
        if out.parts.iter().any(|p| p.node.as_ref() == Some(&v)) {
            continue;
        }
        let (t, e) = stored(store, spec, &v)?.ok_or_else(|| Error::UnknownProgram {
            program: v.to_string(),
        })?;
        walk(store, spec, &v, &t, Some(e), stack, out)?;
    }
    stack.pop();
    out.parts.push(Part {
        node: Some(node.clone()),
        source,
        text: scan::strip_includes(text),
    });
    Ok(())
}

/// A stored program with its includes, in load order (the program last).
// @tg: implements:: [[openspec:elpi-programs#Includes]] {since: "2026-10-31"}
pub fn resolve(store: &dyn ReadStore, spec: &ViewSpec, program: &Value) -> Result<Source> {
    let (text, e) = stored(store, spec, program)?.ok_or_else(|| Error::UnknownProgram {
        program: program.to_string(),
    })?;
    let mut out = Source { parts: Vec::new() };
    walk(
        store,
        spec,
        program,
        &text,
        Some(e),
        &mut Vec::new(),
        &mut out,
    )?;
    Ok(out)
}

/// Program text with its includes resolved from the view; `node` names the
/// program the text will be stored as, so an include of it is a cycle.
pub fn resolve_text(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    text: &str,
    node: Option<&Value>,
) -> Result<Source> {
    let mut out = Source { parts: Vec::new() };
    let root = node
        .cloned()
        .unwrap_or_else(|| Value::iri("urn:tiramemsu:elpi:text"));
    walk(store, spec, &root, text, None, &mut Vec::new(), &mut out)?;
    if node.is_none() {
        if let Some(last) = out.parts.last_mut() {
            last.node = None;
        }
    }
    Ok(out)
}

/// A stored program as a view sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramInfo {
    /// The program node.
    pub program: Value,
    /// Its live `elpi:source` statement.
    pub statement: Eid,
    /// Its text.
    pub text: String,
    /// Every program it includes, directly or not, with its source statement.
    pub includes: Vec<(Value, Eid)>,
    /// True when it or an include may be non-monotonic.
    pub non_monotonic: bool,
    /// True when `(program elpi:enforced true)` is in the view.
    pub enforced: bool,
}

/// What a view knows of a stored program.
// @tg: implements:: [[openspec:elpi-programs#Program information]] {since: "2026-10-31"}
pub fn info(store: &dyn ReadStore, spec: &ViewSpec, program: &Value) -> Result<ProgramInfo> {
    let (text, statement) = stored(store, spec, program)?.ok_or_else(|| Error::UnknownProgram {
        program: program.to_string(),
    })?;
    let src = resolve(store, spec, program)?;
    let enforced = match (
        dict::encode(store, program)?,
        dict::encode(store, &Value::iri(ELPI_ENFORCED))?,
    ) {
        (Some(s), Some(p)) => {
            !tm_store::read::triples(store, spec, Some(s), Some(p), None)?.is_empty()
        }
        _ => false,
    };
    Ok(ProgramInfo {
        program: program.clone(),
        statement,
        text,
        includes: src
            .parts
            .iter()
            .filter(|p| p.node.as_ref() != Some(program))
            .filter_map(|p| Some((p.node.clone()?, p.source?)))
            .collect(),
        non_monotonic: src.non_monotonic(),
        enforced,
    })
}

fn live_eid(tx: &mut Tx<'_>, node: &Value, p: &str) -> Result<Option<(Eid, Value)>> {
    let (Some(s), Some(p)) = (tx.lookup(node)?, tx.lookup(&Value::iri(p))?) else {
        return Ok(None);
    };
    match tx.live_rows(Some(s), Some(p), None)?.pop() {
        Some(row) => {
            let eid = Eid::from_oid(ObjectId::from_raw(row.eid))
                .ok_or_else(|| Error::storage("a statement row without a statement id"))?;
            let v = tx.decode(ObjectId::from_raw(row.o))?;
            Ok(Some((eid, v)))
        }
        None => Ok(None),
    }
}

/// Fails with `RuleLanguageConflict` when `node` holds live rules of another
/// language than `mine` (`n3:source`, `dl:source` or `elpi:source`).
pub fn check_language(tx: &mut Tx<'_>, node: &Value, mine: &str) -> Result<()> {
    for (other, name) in [
        (N3_SOURCE, "N3"),
        (DL_SOURCE, "Datalog"),
        (ELPI_SOURCE, "ELPI"),
    ] {
        if other != mine && live_eid(tx, node, other)?.is_some() {
            return Err(Error::RuleLanguageConflict {
                node: node.to_string(),
                existing: name.into(),
            });
        }
    }
    Ok(())
}

/// Stores ELPI text as program `program`: its one live `elpi:source`
/// statement, superseding other text; equal text writes nothing.
// @tg: implements:: [[openspec:elpi-programs#Store an ELPI program]] {since: "2026-10-31"}
// @tg: implements:: [[openspec:elpi-programs#Rule language conflict]] {since: "2026-10-31"}
// @tg: implements:: [[elpi#Program Nodes With Includes]] {since: "2026-10-31"}
pub fn store_program(tx: &mut Tx<'_>, program: Value, text: &str) -> Result<Eid> {
    let src = on_tx(tx, |s| {
        resolve_text(s, &ViewSpec::NOW, text, Some(&program))
    })?;
    tm_elpi::validate(&src)?;
    check_language(tx, &program, ELPI_SOURCE)?;
    let value = Value::str(text);
    if let Some((eid, current)) = live_eid(tx, &program, ELPI_SOURCE)? {
        if current == value {
            return Ok(eid);
        }
        return tx.supersede(eid, Patch::object(value));
    }
    Ok(tx
        .assert(program, Value::iri(ELPI_SOURCE), value, Valid::ALWAYS)?
        .eid())
}

/// Turns enforcement of stored program `program` on (`(program
/// elpi:enforced true)`, returned) or off (retracted).
// @tg: implements:: [[openspec:elpi-constraints#Enforce an ELPI program]] {since: "2026-10-31"}
pub fn enforce(tx: &mut Tx<'_>, program: Value, on: bool) -> Result<Option<Eid>> {
    let enforced = Value::iri(ELPI_ENFORCED);
    let live = |tx: &mut Tx<'_>| -> Result<Vec<Eid>> {
        let (Some(s), Some(p)) = (tx.lookup(&program)?, tx.lookup(&enforced)?) else {
            return Ok(Vec::new());
        };
        Ok(tx
            .live_rows(Some(s), Some(p), None)?
            .into_iter()
            .filter_map(|r| Eid::from_oid(ObjectId::from_raw(r.eid)))
            .collect())
    };
    if !on {
        for e in live(tx)? {
            tx.retract(e)?;
        }
        return Ok(None);
    }
    if live_eid(tx, &program, ELPI_SOURCE)?.is_none() {
        return Err(Error::UnknownProgram {
            program: program.to_string(),
        });
    }
    if let Some(e) = live(tx)?.first() {
        return Ok(Some(*e));
    }
    Ok(Some(
        tx.assert(program, enforced, Value::Bool(true), Valid::ALWAYS)?
            .eid(),
    ))
}
