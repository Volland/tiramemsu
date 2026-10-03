//! `View::sparql`: SPARQL queries and updates through the `tm-sparql` front end.

use tm_core::{Error, Result, TxOptions};
use tm_exec::{CacheMode, QueryResult};
use tm_ir::Params;
use tm_sparql::env::Env;
use tm_sparql::lower::{QueryForm, QueryPlan};
use tm_sparql::results::{json, nt, RdfTriple, Solutions};
use tm_sparql::Prepared;

use crate::view::View;

/// The result of [`View::sparql`]: which variant you get follows the query form.
/// Use [`SparqlResult::solutions`] for `SELECT`, and `write_sparql_json` or
/// `write_ntriples` for the wire formats.
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
    Ok(Solutions {
        vars,
        rows,
        provenance: None,
        provenance_gaps: Vec::new(),
    })
}

/// Options of [`View::sparql_with`]. Build it with `..Default::default()` so that
/// options added later keep their defaults.
///
/// ```
/// use tiramemsu::SparqlOptions;
///
/// let opts = SparqlOptions {
///     provenance: true,
///     ..Default::default()
/// };
/// assert!(opts.provenance);
/// assert!(!SparqlOptions::default().provenance);
/// assert!(!SparqlOptions::default().query_only);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SparqlOptions {
    /// Attach to each `SELECT` row the eids of the stored statements that matched
    /// to produce it ([`Solutions::provenance`]). Off by default.
    pub provenance: bool,
    /// Refuse an update request with `Unsupported("update in a query-only call")`
    /// before anything runs, so text from an untrusted caller can only read. Off
    /// by default.
    pub query_only: bool,
}

impl View<'_> {
    /// The environment for SPARQL text on this view: the view descriptor, the
    /// database `@vocab` and prefix table, and the start instant for `NOW()`.
    fn sparql_env(&self) -> Result<Env> {
        self.sparql_env_with(None)
    }

    /// [`View::sparql_env`] with the `@vocab` and prefix table given instead of
    /// read from the database (saved answers re-run with the settings they were
    /// saved with).
    pub(crate) fn sparql_env_with(&self, settings: Option<&Settings>) -> Result<Env> {
        let mut env = Env::new(self.descriptor());
        env.speculative = self.db().is_none();
        env.now_ms = match self.db() {
            Some(db) => db.now_ms(),
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
        };
        let (vocab_iri, prefixes) = match settings {
            Some(s) => s.clone(),
            None => self.exec(|e, _| read_settings(e))?,
        };
        if let Some(v) = vocab_iri {
            env.vocab = v;
        }
        env.prefixes = prefixes;
        Ok(env)
    }

    /// Runs SPARQL query text on this view (`SELECT`, `ASK`, `CONSTRUCT`) or an
    /// update request on the plain current view. RDF 1.2 annotations (`{| ... |}`,
    /// `~ ?r`) reach the layers of a fact, and `SERVICE <urn:tiramemsu:tm:asOf/N>`
    /// changes the time of one group. `v:`, `sys:`, `tm:`, `rdf:`, `rdfs:` and `xsd:`
    /// are predeclared. An update is one transaction on the writer.
    ///
    /// # Errors
    ///
    /// `Parse` for invalid text, `Unsupported` for constructs outside the supported
    /// subset (including an update on an as-of, history or valid-at view), `Eval`
    /// for runtime errors, and the write errors of [`crate::Db::transact`] for updates.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// db.now().sparql("INSERT DATA { v:alice v:worksAt v:acme }")?;
    /// let r = db.now().sparql("SELECT ?o WHERE { v:alice v:worksAt ?o }")?;
    /// let solutions = r.solutions().unwrap();
    /// assert_eq!(solutions.vars, ["o"]);
    /// assert_eq!(solutions.rows.len(), 1);
    /// assert!(matches!(db.now().sparql("SELECT ?"), Err(Error::Parse { .. })));
    /// # Ok::<(), Error>(())
    /// ```
    pub fn sparql(&self, text: &str) -> Result<SparqlResult> {
        self.sparql_with(text, &SparqlOptions::default())
    }

    /// Runs SPARQL text like [`View::sparql`], with options.
    ///
    /// With `provenance`, each row of a `SELECT` carries the eids of the stored
    /// statements that matched to produce it, read with [`Solutions::provenance`]:
    /// the patterns of the group, matched `OPTIONAL` parts, the `UNION` branch
    /// taken, annotations, `GRAPH` memberships and `SERVICE` time scopes (an eid
    /// matched in an as-of scope may be retracted now). Statements only tested by
    /// `FILTER EXISTS`, `NOT EXISTS` or `MINUS`, virtual predicates and recursive
    /// property paths add none; a query with a recursive path lists
    /// [`ProvenanceGap::RecursivePath`](crate::ProvenanceGap) in
    /// [`Solutions::provenance_gaps`], and [`Solutions::provenance_complete`] is
    /// then `Some(false)`. The rows themselves are those of [`View::sparql`];
    /// several eids with the same `(s, p, o)` are one SPARQL triple and are all
    /// listed. `DISTINCT` merges rows and unions their provenance, and a group's
    /// provenance is the union over its rows.
    ///
    /// # Errors
    ///
    /// The errors of [`View::sparql`], and with `provenance`:
    /// `Unsupported("provenance for ASK")`, `("provenance for CONSTRUCT")` and
    /// `("provenance for updates")`, before anything runs. With `query_only`, an
    /// update request is `Unsupported("update in a query-only call")`, also before
    /// anything runs.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// db.now().sparql("INSERT DATA { v:alice v:worksAt v:acme . v:acme v:in v:paris }")?;
    /// let opts = SparqlOptions { provenance: true, ..Default::default() };
    /// let r = db.now().sparql_with("SELECT ?c WHERE { v:alice v:worksAt ?o . ?o v:in ?c }", &opts)?;
    /// let s = r.solutions().unwrap();
    /// let cited = s.provenance(0).unwrap();
    /// assert_eq!(cited.len(), 2); // both statements, ascending
    /// assert!(cited[0] < cited[1]);
    /// // without the option there is no provenance
    /// let plain = db.now().sparql("SELECT ?c WHERE { v:alice v:worksAt ?o . ?o v:in ?c }")?;
    /// assert_eq!(plain.solutions().unwrap().provenance(0), None);
    /// // a recursive path leaves its statements uncited
    /// let r = db.now().sparql_with("SELECT ?c WHERE { v:alice (v:worksAt/v:in)+ ?c }", &opts)?;
    /// assert_eq!(r.solutions().unwrap().provenance_complete(), Some(false));
    /// // query-only text cannot write
    /// let read = SparqlOptions { query_only: true, ..Default::default() };
    /// let e = db.now().sparql_with("INSERT DATA { v:a v:p v:b }", &read);
    /// assert!(matches!(e, Err(Error::Unsupported { .. })));
    /// # Ok::<(), Error>(())
    /// ```
    pub fn sparql_with(&self, text: &str, opts: &SparqlOptions) -> Result<SparqlResult> {
        self.op(|| {
            let env = self.sparql_env()?;
            match tm_sparql::prepare(text, &env)? {
                Prepared::Query(plan) if opts.provenance => self.run_provenance(&plan),
                Prepared::Query(plan) => self.run_plan(&plan),
                Prepared::Update(_) if opts.query_only => {
                    Err(Error::unsupported(tm_sparql::error::UPDATE_QUERY_ONLY))
                }
                Prepared::Update(_) if opts.provenance => {
                    Err(Error::unsupported(tm_sparql::error::PROVENANCE_UPDATE))
                }
                Prepared::Update(plan) => self.run_update(&plan),
            }
        })
    }

    /// Runs a `SELECT` with provenance: the instrumented query and its sibling
    /// lookups in one read, so both see the same state, and under one operation
    /// budget, so the lookups draw on what the main query left.
    pub(crate) fn run_provenance(&self, plan: &QueryPlan) -> Result<SparqlResult> {
        let p = tm_sparql::provenance::instrument(plan)?;
        let engine = self.engine()?;
        let mode = if self.db().is_some() {
            CacheMode::Shared
        } else {
            CacheMode::Scoped
        };
        let main = engine.prepare(&p.query, &Params::new())?;
        let sol = self.exec(|e, _| {
            let raw = to_solutions(&engine.execute(&mut *e, mode, &main)?)?;
            p.assemble(raw, &mut |q| {
                let lookup = engine.prepare(q, &Params::new())?;
                to_solutions(&engine.execute(&mut *e, mode, &lookup)?)
            })
        })?;
        Ok(SparqlResult::Solutions(sol))
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

    pub(crate) fn run_plan(&self, plan: &QueryPlan) -> Result<SparqlResult> {
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
