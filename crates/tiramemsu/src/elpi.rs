//! ELPI over a view (`View::elpi`, `View::elpi_check`, `View::elpi_program_info`)
//! and stored programs (`TxElpi`), with the engine of `tm-elpi` and the
//! persistence of `tm-elpi-store` (OpenSpec changes `add-elpi-engine` and
//! `add-elpi-programs`, facade feature `elpi`).

use tm_core::sqlstore::SqlStore;
use tm_core::store::ReadStore;
use tm_core::{Eid, Result, TxHandle as Tx, Value, ViewSpec};
use tm_elpi::{CheckResult, ConcludeResult, ElpiOptions, QueryResult, Source};
use tm_elpi_store::ProgramInfo;

use crate::view::View;

/// An ELPI program to run: text (its `accumulate <iri>.` lines resolved from
/// the view) or a stored program node.
#[derive(Clone, Debug, PartialEq)]
pub enum ElpiProgram {
    /// Program text.
    Text(String),
    /// A stored program (`(node elpi:source "<text>")`).
    Stored(Value),
}

fn source(store: &dyn ReadStore, spec: &ViewSpec, program: &ElpiProgram) -> Result<Source> {
    match program {
        ElpiProgram::Text(t) => tm_elpi_store::resolve_text(store, spec, t, None),
        ElpiProgram::Stored(v) => tm_elpi_store::resolve(store, spec, v),
    }
}

fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl View<'_> {
    /// Runs `f` on this view's `ReadStore` with the pinned clock.
    pub(crate) fn with_elpi_store<R>(
        &self,
        f: impl Fn(&dyn ReadStore, i64) -> Result<R>,
    ) -> Result<R> {
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

    /// Solves an ELPI goal over this view and returns one row per solution
    /// (feature `elpi`); writes nothing.
    ///
    /// The program reads statements through `tm.triple S P O E`, valid at one
    /// instant: the view's valid-at instant, else `opts.valid_at`, else the
    /// clock. Each row binds the goal's named variables (those not starting
    /// with `_`).
    ///
    /// # Errors
    ///
    /// `ElpiProgram` for text that does not load, `ElpiRun` for a runtime
    /// error, `ElpiOutOfFuel` when the fuel or row budget runs out,
    /// `UnknownProgram` and `ProgramCycle` for stored programs and includes.
    // @tg: implements:: [[openspec:elpi-engine#Goal queries over a view]] {since: "2026-10-31"}
    // @tg: implements:: [[openspec:elpi-engine#Optional ELPI feature]] {since: "2026-10-31"}
    pub fn elpi(
        &self,
        program: &ElpiProgram,
        goal: &str,
        opts: &ElpiOptions,
    ) -> Result<QueryResult> {
        let spec = self.spec();
        self.with_elpi_store(|s, clock| {
            let src = source(s, &spec, program)?;
            tm_elpi::query(s, &spec, clock, &src, goal, opts)
        })
    }

    /// Every violation of the program's `tm.violation Sev Name Msg Eids` over
    /// every statement of this view (feature `elpi`).
    // @tg: implements:: [[openspec:elpi-engine#Full constraint check]] {since: "2026-10-31"}
    pub fn elpi_check(&self, program: &ElpiProgram, opts: &ElpiOptions) -> Result<CheckResult> {
        let spec = self.spec();
        self.with_elpi_store(|s, _| {
            let src = source(s, &spec, program)?;
            tm_elpi::check(s, &spec, &src, None, opts)
        })
    }

    /// Every conclusion of the program's `tm.conclude Fact Support` over this
    /// view, with the support it states and the interval it holds over
    /// (feature `elpi`); writes nothing. `Db::materialize_elpi` stores them.
    // @tg: implements:: [[openspec:elpi-engine#Conclusions carry stated support]] {since: "2026-10-31"}
    pub fn elpi_conclude(
        &self,
        program: &ElpiProgram,
        opts: &ElpiOptions,
    ) -> Result<ConcludeResult> {
        let spec = self.spec();
        self.with_elpi_store(|s, _| {
            let src = source(s, &spec, program)?;
            tm_elpi::conclude(s, &spec, &src, opts)
        })
    }

    /// A stored ELPI program as this view sees it: text, source statement,
    /// included programs, monotonicity label and enforcement (feature `elpi`).
    pub fn elpi_program_info(&self, program: &Value) -> Result<ProgramInfo> {
        let spec = self.spec();
        self.with_elpi_store(|s, _| tm_elpi_store::info(s, &spec, program))
    }
}

/// ELPI programs inside a transaction (feature `elpi`).
pub trait TxElpi {
    /// Stores ELPI text as program `program`: the one live statement
    /// `(program elpi:source "<text>")`. The text and its includes are loaded
    /// first; other live text is superseded, equal text writes nothing.
    ///
    /// # Errors
    ///
    /// `ElpiProgram` (nothing is written), `ProgramCycle`, `UnknownProgram` for
    /// a missing include, `RuleLanguageConflict` when the node holds N3 rules or
    /// a Datalog program.
    fn store_elpi_program(&mut self, program: Value, text: &str) -> Result<Eid>;

    /// Turns enforcement of stored ELPI program `program` on (returns
    /// `(program elpi:enforced true)`) or off. At this transaction's commit the
    /// full `tm.violation` check runs; later commits run `tm.violation-delta`.
    ///
    /// # Errors
    ///
    /// `UnknownProgram`.
    fn enforce_elpi(&mut self, program: Value, on: bool) -> Result<Option<Eid>>;
}

impl TxElpi for Tx<'_> {
    fn store_elpi_program(&mut self, program: Value, text: &str) -> Result<Eid> {
        tm_elpi_store::store_program(self, program, text)
    }

    fn enforce_elpi(&mut self, program: Value, on: bool) -> Result<Option<Eid>> {
        tm_elpi_store::enforce(self, program, on)
    }
}
