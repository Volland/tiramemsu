//! `View::sparql`: SPARQL queries and updates through the `tm-sparql` front end.

use tm_core::{Error, Result, TxOptions};
use tm_exec::{CacheMode, QueryResult};
use tm_ir::Params;
use tm_sparql::env::Env;
use tm_sparql::lower::{QueryForm, QueryPlan};
use tm_sparql::results::{json, nt, RdfTriple, Solutions};
use tm_sparql::Prepared;

use crate::view::View;

/// The result of [`View::sparql`].
#[derive(Clone, Debug, PartialEq)]
pub enum SparqlResult {
    /// A `SELECT` result.
    Solutions(Solutions),
    /// An `ASK` answer.
    Boolean(bool),
    /// A `CONSTRUCT` graph.
    Graph(Vec<RdfTriple>),
    /// The report of an update request.
    Update(tm_core::TxReport),
}

impl SparqlResult {
    /// The SPARQL 1.1 Query Results JSON of a `SELECT` or `ASK`.
    pub fn write_sparql_json(&self) -> Result<String> {
        match self {
            SparqlResult::Solutions(s) => Ok(json::write_select(s)),
            SparqlResult::Boolean(b) => Ok(json::write_ask(*b)),
            _ => Err(Error::unsupported(
                "SPARQL JSON for a graph or update result",
            )),
        }
    }

    /// N-Triples (RDF 1.2 N-Triples when a triple term occurs) of a `CONSTRUCT`.
    pub fn write_ntriples(&self) -> Result<String> {
        match self {
            SparqlResult::Graph(g) => Ok(nt::write(g)),
            _ => Err(Error::unsupported("N-Triples for a non-graph result")),
        }
    }

    /// The solutions of a `SELECT`.
    pub fn solutions(&self) -> Option<&Solutions> {
        match self {
            SparqlResult::Solutions(s) => Some(s),
            _ => None,
        }
    }
}

fn to_solutions(r: &QueryResult) -> Result<Solutions> {
    let vars = r.columns.iter().map(|v| v.name().to_string()).collect();
    let mut rows = Vec::with_capacity(r.rows.len());
    for row in &r.rows {
        let mut out = Vec::with_capacity(row.len());
        for cell in row {
            out.push(match cell {
                None => None,
                Some(c) => match c.as_term() {
                    Some(v) => Some(v.clone()),
                    None => return Err(Error::unsupported("a list value in a SPARQL result")),
                },
            });
        }
        rows.push(out);
    }
    Ok(Solutions { vars, rows })
}

impl View<'_> {
    /// The environment for SPARQL text on this view: the view descriptor, the
    /// database `@vocab` and prefix table, and the start instant for `NOW()`.
    fn sparql_env(&self) -> Result<Env> {
        let mut env = Env::new(self.descriptor());
        env.speculative = self.db().is_none();
        env.now_ms = match self.db() {
            Some(db) => db.now_ms(),
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
        };
        let (vocab_iri, prefixes) = self.exec(|e, _| read_settings(e))?;
        if let Some(v) = vocab_iri {
            env.vocab = v;
        }
        env.prefixes = prefixes;
        Ok(env)
    }

    /// Runs SPARQL query text on this view (`SELECT`, `ASK`, `CONSTRUCT`) or an
    /// update request on the plain current view.
    pub fn sparql(&self, text: &str) -> Result<SparqlResult> {
        let env = self.sparql_env()?;
        match tm_sparql::prepare(text, &env)? {
            Prepared::Query(plan) => self.run_plan(&plan),
            Prepared::Update(plan) => self.run_update(&plan),
        }
    }

    /// Runs a checked update: the whole request is one transaction on the writer.
    fn run_update(&self, plan: &tm_sparql::update::UpdatePlan) -> Result<SparqlResult> {
        let db = self
            .db()
            .ok_or_else(|| Error::unsupported(tm_sparql::error::UPDATE_NON_CURRENT))?;
        let engine = self.engine()?;
        let report = db.transact(TxOptions::default(), |tx| {
            tm_sparql::update::run(plan, tx, &mut |tx, q| {
                let p = engine.prepare(q, &Params::new())?;
                let r = tx.read_with(|e| engine.execute(e, CacheMode::Scoped, &p))?;
                to_solutions(&r)
            })
        })?;
        Ok(SparqlResult::Update(report))
    }

    fn run_plan(&self, plan: &QueryPlan) -> Result<SparqlResult> {
        let r = self.execute_ir(&plan.query, &Params::new())?;
        let sol = to_solutions(&r)?;
        Ok(match &plan.form {
            QueryForm::Select => SparqlResult::Solutions(sol),
            QueryForm::Ask => SparqlResult::Boolean(!sol.rows.is_empty()),
            QueryForm::Construct { template } => {
                SparqlResult::Graph(tm_sparql::construct::instantiate(template, &sol)?)
            }
        })
    }
}

pub(crate) use tm_core::mapping::{read_settings, Settings};
