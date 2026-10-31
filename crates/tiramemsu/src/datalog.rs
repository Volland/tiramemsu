//! Datalog over a view (`View::datalog`, `View::datalog_reason`, `View::check`,
//! `View::program_info`, `View::explain_datalog`) and stored programs
//! (`TxDatalog::store_program`), with the engine of `tm-datalog` (OpenSpec change
//! `add-datalog-engine`, facade feature `datalog`).
//!
//! Both engines hand the run a `ReadStore`: redb its snapshot, SQLite a
//! [`SqlStore`] over the view's connection, so the same code reads either file.

use tm_core::sqlstore::SqlStore;
use tm_core::store::ReadStore;
use tm_core::vocab::{DL_SOURCE, N3_SOURCE};
use tm_core::{Eid, Error, ObjectId, Patch, Result, TxHandle as Tx, Valid, Value};
use tm_datalog::{
    CheckResult, DatalogOptions, DatalogResult, Explain, Program, ProgramInfo, ReasonResult,
};

use crate::view::View;

fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl View<'_> {
    // @tg: implements:: [[openspec:optional-query-frontends#Optional Datalog feature]] {since: "2026-10-28"}
    pub(crate) fn with_store_pub<R>(
        &self,
        f: impl Fn(&dyn ReadStore, i64) -> Result<R>,
    ) -> Result<R> {
        self.with_store(f)
    }

    fn with_store<R>(&self, f: impl Fn(&dyn ReadStore, i64) -> Result<R>) -> Result<R> {
        let clock = self.db().map_or_else(system_now, |db| db.now_ms());
        self.op(|| {
            self.either(
                |e, _| {
                    let s = SqlStore::new(e);
                    f(&s, clock)
                },
                |r, _, _| f(r, clock),
            )
        })
    }

    /// Evaluates a Datalog program over this view and returns the rows of a goal
    /// (feature `datalog`); writes nothing.
    ///
    /// `goal` is a body such as `reports(ex:alice, X)` (with or without `?-`),
    /// read with the program's prefixes; `None` runs the first goal of the text.
    /// Each row binds the goal's named variables and carries the valid time over
    /// which the binding follows from the statements valid then. A goal with
    /// constants is evaluated for demand (magic sets) unless `opts.demand` is off.
    ///
    /// # Errors
    ///
    /// `DatalogParse`, the analysis errors (`DatalogUnsafe`,
    /// `DatalogUnstratifiable`, `DatalogUnboundedRecursion`, ...),
    /// `UnknownProgram`, `DatalogIncomplete` with `require_complete`, and the
    /// budget and read errors.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("alice"), v("manager"), v("bob"), Valid::ALWAYS)?;
    ///     tx.assert(v("bob"), v("manager"), v("carol"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let program = DatalogProgram::Text(
    ///     "reports(X, Y) :- v:manager(X, Y).\n\
    ///      reports(X, Z) :- reports(X, Y), v:manager(Y, Z).".into(),
    /// );
    /// let r = db.now().datalog(&program, Some("reports(v:alice, X)"), &DatalogOptions::default())?;
    /// assert_eq!(r.rows.len(), 2);
    /// assert_eq!(r.rows[0].values, [DatalogTerm::Value(v("bob"))]);
    /// # Ok::<(), Error>(())
    /// ```
    // @tg: implements:: [[openspec:datalog-evaluation#Goal queries over a view]] {since: "2026-10-28"}
    pub fn datalog(
        &self,
        program: &Program,
        goal: Option<&str>,
        opts: &DatalogOptions,
    ) -> Result<DatalogResult> {
        let spec = self.spec();
        self.with_store(|s, clock| tm_datalog::query(s, &spec, clock, program, goal, opts))
    }

    /// Every conclusion of a program's IRI relations that is not among the
    /// stored statements, one per maximal interval, in the shape of N3's
    /// conclusions (feature `datalog`).
    // @tg: implements:: [[openspec:datalog-evaluation#Full reasoning over a view]] {since: "2026-10-28"}
    pub fn datalog_reason(&self, program: &Program, opts: &DatalogOptions) -> Result<ReasonResult> {
        let spec = self.spec();
        self.with_store(|s, clock| tm_datalog::reason(s, &spec, clock, program, opts))
    }

    /// Runs a goal and returns the plan beside the result: strata, iterations,
    /// each rule variant's order and each stored predicate's access (feature
    /// `datalog`).
    // @tg: implements:: [[openspec:datalog-evaluation#Explain a program run]] {since: "2026-10-28"}
    pub fn explain_datalog(
        &self,
        program: &Program,
        goal: Option<&str>,
        opts: &DatalogOptions,
    ) -> Result<Explain> {
        let spec = self.spec();
        self.with_store(|s, clock| tm_datalog::explain(s, &spec, clock, program, goal, opts))
    }

    /// Evaluates a program's constraints over this view (those in `only`, or
    /// all) and returns the violated ones, each violation with its bindings,
    /// the statements its derivation used and when it holds (feature `datalog`).
    // @tg: implements:: [[openspec:datalog-constraints#Check constraints]] {since: "2026-10-28"}
    pub fn check(
        &self,
        program: &Program,
        only: &[String],
        opts: &DatalogOptions,
    ) -> Result<CheckResult> {
        let spec = self.spec();
        self.with_store(|s, clock| tm_datalog::check(s, &spec, clock, program, only, opts))
    }

    /// A program as this view sees it: its text, rules, goals, constraints,
    /// strata and the stored predicates it reads (feature `datalog`).
    // @tg: implements:: [[openspec:datalog-programs#Program information]] {since: "2026-10-28"}
    pub fn program_info(&self, program: &Program) -> Result<ProgramInfo> {
        let spec = self.spec();
        self.with_store(|s, _| tm_datalog::info(s, &spec, program))
    }
}

/// Datalog programs inside a transaction (feature `datalog`).
pub trait TxDatalog {
    /// Stores Datalog text as program `program`: the one live statement
    /// `(program dl:source "<text>")`. The text is parsed and analysed first; a
    /// live source with other text is superseded, so `as_of` views run the
    /// program believed then; equal text returns the live statement and writes
    /// nothing. Returns the live statement's eid.
    ///
    /// # Errors
    ///
    /// The parse and analysis errors (nothing is written), `RuleLanguageConflict`
    /// when the node holds live N3 rules, and the write errors.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.store_program(v("reporting"), "reports(X, Y) :- v:manager(X, Y).")?;
    ///     Ok(())
    /// })?;
    /// let info = db.now().program_info(&DatalogProgram::Stored(v("reporting")))?;
    /// assert_eq!(info.analysis.rules, 1);
    /// # Ok::<(), Error>(())
    /// ```
    fn store_program(&mut self, program: Value, text: &str) -> Result<Eid>;

    /// Turns enforcement of stored program `program` on (`(program dl:enforced
    /// true)`, returned) or off (retracted). From the next commit on, and for
    /// this transaction itself, every commit is checked against the program's
    /// constraints: an `error` violation the commit introduces rolls it back with
    /// `ConstraintViolated`; `warn` violations are listed in its report. Turning
    /// enforcement on checks the whole program at commit.
    ///
    /// # Errors
    ///
    /// `UnknownProgram`, and `ConstraintNotEnforceable` for an `error` constraint
    /// over a recursive relation, an aggregate, `since`/`until` or an operator
    /// with an infinite window.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
    /// db.transact(TxOptions::default(), |tx| {
    ///     tx.store_program(v("policies"), "constraint one error :- v:role(X, v:CEO), v:role(Y, v:CEO), X != Y.")?;
    ///     tx.enforce(v("policies"), true)?;
    ///     tx.assert(v("a"), v("role"), v("CEO"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let second = db.transact(TxOptions::default(), |tx| {
    ///     tx.assert(v("b"), v("role"), v("CEO"), Valid::ALWAYS)?;
    ///     Ok(())
    /// });
    /// assert!(matches!(second, Err(Error::ConstraintViolated { .. })));
    /// # Ok::<(), Error>(())
    /// ```
    fn enforce(&mut self, program: Value, on: bool) -> Result<Option<Eid>>;
}

/// The live statement `(node p ?)`, if any.
fn live_source(tx: &mut Tx<'_>, node: &Value, p: &str) -> Result<Option<(Eid, Value)>> {
    let (Some(s), Some(p)) = (tx.lookup(node)?, tx.lookup(&Value::iri(p))?) else {
        return Ok(None);
    };
    match tx.live_rows(Some(s), Some(p), None)?.pop() {
        Some(row) => {
            let eid = Eid::from_oid(ObjectId::from_raw(row.eid))
                .ok_or_else(|| Error::storage("a statement row without a statement id"))?;
            let text = tx.decode(ObjectId::from_raw(row.o))?;
            Ok(Some((eid, text)))
        }
        None => Ok(None),
    }
}

/// Fails with `RuleLanguageConflict` when `node` holds live rules of the other
/// language (`other` is `n3:source`, `dl:source` or `elpi:source`).
pub(crate) fn check_language(tx: &mut Tx<'_>, node: &Value, other: &str) -> Result<()> {
    if live_source(tx, node, other)?.is_some() {
        return Err(Error::RuleLanguageConflict {
            node: node.to_string(),
            existing: match other {
                N3_SOURCE => "N3",
                tm_core::vocab::ELPI_SOURCE => "ELPI",
                _ => "Datalog",
            }
            .into(),
        });
    }
    Ok(())
}

impl TxDatalog for Tx<'_> {
    // @tg: implements:: [[openspec:datalog-programs#Store a program]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:datalog-programs#Programs have transaction time]] {since: "2026-10-28"}
    // @tg: implements:: [[openspec:datalog-programs#Rule language conflict]] {since: "2026-10-28"}
    fn store_program(&mut self, program: Value, text: &str) -> Result<Eid> {
        tm_datalog::validate(text)?;
        check_language(self, &program, N3_SOURCE)?;
        check_language(self, &program, tm_core::vocab::ELPI_SOURCE)?;
        let value = Value::str(text);
        if let Some((eid, current)) = live_source(self, &program, DL_SOURCE)? {
            if current == value {
                return Ok(eid);
            }
            return self.supersede(eid, Patch::object(value));
        }
        Ok(self
            .assert(program, Value::iri(DL_SOURCE), value, Valid::ALWAYS)?
            .eid())
    }

    fn enforce(&mut self, program: Value, on: bool) -> Result<Option<Eid>> {
        crate::datalog_enforce::enforce(self, program, on)
    }
}
