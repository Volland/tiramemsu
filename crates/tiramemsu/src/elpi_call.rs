//! The ELPI provider of the query engine (`lat.md/elpi#Own Calls`): a thin
//! wrapper over `tm_elpi::query` that the facade registers when it is built with
//! `exec` and `elpi`, so a SPARQL `SERVICE <urn:tiramemsu:elpi:query>` or a
//! Cypher `CALL tiramemsu.elpi.query` returns the rows of `View::elpi`.

use std::sync::Arc;

use tm_core::store::ReadStore;
use tm_core::{Clock, Result, Valid, ViewSpec};
use tm_elpi::{ElpiOptions, ElpiTerm};
use tm_exec::datalog::{DatalogProvider, DlCell, DlProgram, DlRow, DlRows};

/// Runs ELPI goals with the engine of `tm-elpi`; a now view pins `clock`.
pub(crate) struct Provider {
    pub(crate) clock: Arc<dyn Clock>,
}

impl DatalogProvider for Provider {
    // @tg: implements:: [[openspec:elpi-query-calls#ELPI goal calls]] {since: "2026-10-31"}
    fn goal(
        &self,
        store: &dyn ReadStore,
        view: &ViewSpec,
        program: &DlProgram,
        goal: &str,
    ) -> Result<DlRows> {
        let src = match program {
            DlProgram::Stored(iri) => {
                tm_elpi_store::resolve(store, view, &tm_core::Value::iri(iri.clone()))?
            }
            DlProgram::Text(t) => tm_elpi_store::resolve_text(store, view, t, None)?,
        };
        let r = tm_elpi::query(
            store,
            view,
            self.clock.now_ms(),
            &src,
            goal,
            &ElpiOptions::default(),
        )?;
        // the columns are the goal's variables as the front ends read them
        let columns = tm_ir::goal_variables(goal);
        let rows = r
            .rows
            .into_iter()
            .map(|row| DlRow {
                values: columns
                    .iter()
                    .map(|c| match row.bindings.get(c) {
                        Some(ElpiTerm::Value(v)) => DlCell::Value(v.clone()),
                        Some(ElpiTerm::Other(o)) => DlCell::Unsupported(o.clone()),
                        None => DlCell::Unsupported(format!("{c} (unbound)")),
                    })
                    .collect(),
                // ELPI goals read one valid instant: the binding has no interval
                valid: vec![Valid::ALWAYS],
            })
            .collect();
        Ok(DlRows { columns, rows })
    }
}
