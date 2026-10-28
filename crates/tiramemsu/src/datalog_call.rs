//! The Datalog provider of the query engine (`lat.md/query#Datalog Calls`): a thin
//! wrapper over `tm_datalog::query` that the facade registers when it is built with
//! `exec` and `datalog`, so a SPARQL `SERVICE <urn:tiramemsu:dl:query>` or a Cypher
//! `CALL tiramemsu.datalog.query` returns the rows of `View::datalog`.

use std::sync::Arc;

use tm_core::store::ReadStore;
use tm_core::{Clock, Result, Value, ViewSpec};
use tm_datalog::{DatalogOptions, Program, Term};
use tm_exec::datalog::{DatalogProvider, DlCell, DlProgram, DlRow, DlRows};

/// The goal text being counted and its evaluations (`None`: not counting).
static WATCH: std::sync::Mutex<Option<(String, usize)>> = std::sync::Mutex::new(None);

/// Starts counting the evaluations of calls whose goal text is `goal`, from zero.
/// A test hook: it shows that a call runs once per query.
#[doc(hidden)]
pub fn watch_datalog_calls(goal: &str) {
    *WATCH.lock().unwrap_or_else(|e| e.into_inner()) = Some((goal.to_string(), 0));
}

/// The evaluations counted since [`watch_datalog_calls`].
#[doc(hidden)]
pub fn watched_datalog_calls() -> usize {
    WATCH
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map_or(0, |(_, n)| *n)
}

/// Runs goals with the engine of `tm-datalog`; a now view pins `clock`.
pub(crate) struct Provider {
    pub(crate) clock: Arc<dyn Clock>,
}

impl DatalogProvider for Provider {
    // @tg: implements:: [[openspec:datalog-query-calls#Same rows as the Rust call]] {since: "2026-10-28"}
    fn goal(
        &self,
        store: &dyn ReadStore,
        view: &ViewSpec,
        program: &DlProgram,
        goal: &str,
    ) -> Result<DlRows> {
        if let Some((g, n)) = WATCH.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            if g == goal {
                *n += 1;
            }
        }
        let program = match program {
            DlProgram::Stored(iri) => Program::Stored(Value::iri(iri.clone())),
            DlProgram::Text(t) => Program::Text(t.clone()),
        };
        let r = tm_datalog::query(
            store,
            view,
            self.clock.now_ms(),
            &program,
            Some(goal),
            &DatalogOptions::default(),
        )?;
        let rows = r
            .rows
            .into_iter()
            .map(|row| DlRow {
                values: row
                    .values
                    .into_iter()
                    .map(|t| match t {
                        Term::Value(v) => DlCell::Value(v),
                        Term::Invented(n) => DlCell::Unsupported(format!("_:invented{n}")),
                    })
                    .collect(),
                valid: row.valid,
            })
            .collect();
        Ok(DlRows {
            columns: r.columns,
            rows,
        })
    }
}
