//! Datalog goal calls (`lat.md/query#Datalog Calls`): the provider trait the query
//! engine calls, and the `tm_datalog` table function that runs it on SQLite.
//!
//! `tm-exec` does not depend on `tm-datalog`: the facade registers a
//! [`DatalogProvider`] when it is built with `exec` and `datalog`, and without one
//! a call fails with `Unsupported` at execution. `tm_datalog(spec)` takes a JSON
//! spec (`{"program":{"iri":…}|{"text":…},"goal":…,"view":…,"outputs":[…]}`) and
//! returns the columns `c0 … c15` (the goal variables named by `outputs`, in
//! order), `validFrom` and `validTo`, one row per goal row and maximal interval.
//! Every cell is an `ObjectId`; a value the store has no id for fails the call.

use std::sync::Arc;

use tm_core::codec::{self, Encoded};
use tm_core::sqlstore::SqlStore;
use tm_core::store::ReadStore;
use tm_core::term::TermKey;
use tm_core::{
    budget, ConnTableFunction, Error, Executor, ObjectId, Result, Tag, Valid, Value, ViewSpec,
};
use tm_core::{SqlError, SqlValue};

use crate::path::view::parse_view_arg;
use crate::plan::{PDatalog, PDlProgram};

/// The SQL name.
pub const NAME: &str = "tm_datalog";

/// The most goal variables a call can bind (the output columns `c0 … c15`).
pub const MAX_OUTPUTS: usize = 16;

/// The program a provider runs.
#[derive(Clone, Debug, PartialEq)]
pub enum DlProgram {
    /// The program stored on this node (IRI).
    Stored(String),
    /// Program text.
    Text(String),
}

/// One cell of a goal row.
#[derive(Clone, Debug, PartialEq)]
pub enum DlCell {
    /// A value.
    Value(Value),
    /// A cell no query row can hold (an invented node), with a description.
    Unsupported(String),
}

/// One goal row: its cells (one per column of the result) and its maximal valid
/// intervals, ascending.
#[derive(Clone, Debug, PartialEq)]
pub struct DlRow {
    /// One cell per column.
    pub values: Vec<DlCell>,
    /// The valid time of the binding.
    pub valid: Vec<Valid>,
}

/// The rows of a goal call.
#[derive(Clone, Debug, PartialEq)]
pub struct DlRows {
    /// The goal's variable names.
    pub columns: Vec<String>,
    /// The rows.
    pub rows: Vec<DlRow>,
}

/// Runs Datalog goals for the query engine. The facade implements it over
/// `tm_datalog::query` when built with `exec` and `datalog`.
pub trait DatalogProvider: Send + Sync {
    /// Evaluates `goal` of `program` over the view `view` of `store`.
    ///
    /// # Errors
    ///
    /// The Datalog errors and the budget errors of the run.
    fn goal(
        &self,
        store: &dyn ReadStore,
        view: &ViewSpec,
        program: &DlProgram,
        goal: &str,
    ) -> Result<DlRows>;
}

/// The error of a call without a registered provider.
// @tg: implements:: [[openspec:datalog-query-calls#Feature gating for calls]] {since: "2026-10-28"}
pub fn no_provider() -> Error {
    Error::Unsupported {
        feature: "Datalog goal call (this build has no Datalog provider: enable the `datalog` \
                  feature of tiramemsu)"
            .to_string(),
    }
}

fn shape(msg: impl std::fmt::Display) -> Error {
    Error::DatalogCallShape {
        message: msg.to_string(),
    }
}

/// The id of a cell in the store's term dictionary or inline, never inserting.
///
/// # Errors
///
/// `Unsupported` when the store has no id for the value (a computed string that
/// is not stored, a node a rule invented), since a query row holds only ids.
pub fn encode_cell(store: &dyn ReadStore, cell: &DlCell) -> Result<ObjectId> {
    let v = match cell {
        DlCell::Value(v) => v,
        DlCell::Unsupported(what) => {
            return Err(Error::Unsupported {
                feature: format!("Datalog call result {what}: a query row cannot hold it"),
            })
        }
    };
    let id = match codec::encode(v) {
        Encoded::Inline(id) => {
            id.check_origin()?;
            Some(id)
        }
        Encoded::Term(spec) => {
            let dt = match &spec.datatype {
                None => Some(None),
                Some(iri) => store
                    .term_id(&TermKey {
                        tag: Tag::Iri as u8,
                        lex: iri.clone(),
                        dt: None,
                        lang: None,
                    })?
                    .map(|id| Some(ObjectId::from_unsigned(Tag::Iri, id as u64))),
            };
            match dt {
                None => None,
                Some(dt) => store
                    .term_id(&TermKey {
                        tag: spec.tag as u8,
                        lex: spec.lex.clone(),
                        dt: dt.map(ObjectId::raw),
                        lang: spec.lang.clone(),
                    })?
                    .map(|id| ObjectId::from_unsigned(spec.tag, id as u64)),
            }
        }
    };
    id.ok_or_else(|| Error::Unsupported {
        feature: format!(
            "Datalog call result {v}: the value is not in the term dictionary, so a query row \
             cannot hold it"
        ),
    })
}

/// One output row of a call: the ids of the requested goal variables and the
/// interval, before it is split into the `validFrom` / `validTo` columns.
pub struct CallRow {
    /// The requested goal variables, in the order asked.
    pub ids: Vec<ObjectId>,
    /// The interval start (`None`: open).
    pub from: Option<ObjectId>,
    /// The interval end (`None`: open).
    pub to: Option<ObjectId>,
}

/// Runs a planned call through `provider` and encodes its rows: one per goal row
/// and maximal interval, in the provider's order.
///
/// # Errors
///
/// `DatalogCallShape` when a requested variable is not a goal variable or the
/// call binds too many; `Unsupported` for a value without an id; the provider's
/// errors.
// @tg: implements:: [[openspec:datalog-query-calls#Budgets apply to calls]] {since: "2026-10-28"}
pub fn run_call(
    provider: &dyn DatalogProvider,
    store: &dyn ReadStore,
    spec: &ViewSpec,
    program: &DlProgram,
    goal: &str,
    outputs: &[String],
) -> Result<Vec<CallRow>> {
    if outputs.len() > MAX_OUTPUTS {
        return Err(shape(format!(
            "a call binds at most {MAX_OUTPUTS} goal variables, got {}",
            outputs.len()
        )));
    }
    budget::check()?;
    let res = provider.goal(store, spec, program, goal)?;
    let mut idx = Vec::with_capacity(outputs.len());
    for name in outputs {
        match res.columns.iter().position(|c| c == name) {
            Some(i) => idx.push(i),
            None => {
                return Err(shape(format!(
                    "`{name}` is not a variable of the goal (its variables: {})",
                    res.columns.join(", ")
                )))
            }
        }
    }
    let date = |ms: Option<i64>| -> Result<Option<ObjectId>> {
        ms.map(|ms| encode_cell(store, &DlCell::Value(Value::DateTime { ms, tz: Some(0) })))
            .transpose()
    };
    let mut out = Vec::new();
    for row in &res.rows {
        budget::check()?;
        let mut ids = Vec::with_capacity(idx.len());
        for &i in &idx {
            ids.push(encode_cell(store, &row.values[i])?);
        }
        for iv in &row.valid {
            out.push(CallRow {
                ids: ids.clone(),
                from: date(iv.from)?,
                to: date(iv.to)?,
            });
        }
    }
    Ok(out)
}

/// The JSON spec of a planned call.
pub fn spec_text(p: &PDatalog) -> String {
    let program = match &p.program {
        PDlProgram::Stored(i) => serde_json::json!({ "iri": i }),
        PDlProgram::Text(t) => serde_json::json!({ "text": t }),
    };
    let outputs: Vec<&str> = p.outputs.iter().map(|(g, _)| g.as_str()).collect();
    serde_json::json!({
        "program": program,
        "goal": p.goal,
        "view": p.view_text,
        "outputs": outputs,
    })
    .to_string()
}

fn arg_err(msg: impl std::fmt::Display) -> Error {
    Error::Sqlite(SqlError::new(
        SqlError::ERROR,
        format!("{NAME}: spec: {msg}"),
    ))
}

/// Evaluates one call of `tm_datalog(spec)` on the calling connection.
// @lat: [[query#Datalog Calls]]
pub fn call(
    provider: Option<&Arc<dyn DatalogProvider>>,
    exec: &mut dyn Executor,
    args: &[SqlValue],
) -> Result<Vec<Vec<SqlValue>>> {
    let Some(SqlValue::Text(text)) = args.first() else {
        return Err(arg_err("expected spec text"));
    };
    let v: serde_json::Value = serde_json::from_str(text).map_err(arg_err)?;
    let program = match (v["program"]["iri"].as_str(), v["program"]["text"].as_str()) {
        (Some(i), None) => DlProgram::Stored(i.to_string()),
        (None, Some(t)) => DlProgram::Text(t.to_string()),
        _ => return Err(arg_err("program is {iri} or {text}")),
    };
    let goal = v["goal"].as_str().ok_or_else(|| arg_err("missing goal"))?;
    let view = v["view"].as_str().unwrap_or("now");
    let (spec, _) = parse_view_arg(view).map_err(arg_err)?;
    let outputs: Vec<String> = v["outputs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let provider = provider.ok_or_else(no_provider)?;
    let store = SqlStore::new(exec);
    let rows = run_call(provider.as_ref(), &store, &spec, &program, goal, &outputs)?;
    let cell = |id: Option<ObjectId>| id.map_or(SqlValue::Null, |i| SqlValue::Integer(i.raw()));
    Ok(rows
        .into_iter()
        .map(|r| {
            let mut row = vec![SqlValue::Null; MAX_OUTPUTS + 2];
            for (i, id) in r.ids.iter().enumerate() {
                row[i] = SqlValue::Integer(id.raw());
            }
            row[MAX_OUTPUTS] = cell(r.from);
            row[MAX_OUTPUTS + 1] = cell(r.to);
            row
        })
        .collect())
}

/// The table function registered on every connection by [`crate::host::install`]:
/// without a provider a call fails with `Unsupported`.
pub fn table_function(provider: Option<Arc<dyn DatalogProvider>>) -> ConnTableFunction {
    let mut columns: Vec<String> = (0..MAX_OUTPUTS).map(|i| format!("c{i}")).collect();
    columns.extend(["validFrom".to_string(), "validTo".to_string()]);
    ConnTableFunction {
        name: NAME.to_string(),
        args: vec!["spec".to_string()],
        columns,
        pushdown: Vec::new(),
        func: Arc::new(move |exec, args| call(provider.as_ref(), exec, args)),
    }
}
