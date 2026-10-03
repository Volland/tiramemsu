//! Saved answers and conservative invalidation (OpenSpec change
//! `add-saved-answer-invalidation`): a query, its parameters, view and settings,
//! its last result with the statements it cited, and an event cursor that turns
//! later events into `recheck` and `stale` marks.
//!
//! The records live in the format-3 tables `saved_answer` and `saved_answer_dep`
//! (`lat.md/storage#Saved Answers`). They are derived: writing them takes no
//! transaction number and logs no event, and graph history is never touched.

use serde_json::{json, Map, Value as J};
use tm_core::{
    read, Eid, Error, Event, Executor, ObjectId, Op, Result, RetKind, SqlValue, TimeRef, TxId,
    TxSel, ValidSel, Value, ViewSpec,
};
use tm_cypher::{CypherParams, CypherValue};
use tm_exec::virtual_pred::VirtualPred;
use tm_ir::expr::Expr;
use tm_ir::path::PathExpr;
use tm_ir::{IrQuery, TermOrVar};
use tm_sparql::lower::QueryForm;
use tm_sparql::results::Solutions;
use tm_sparql::Prepared;

use crate::budget::QueryBudget;
use crate::cypher::CypherTrace;
use crate::db::Db;
use crate::sparql::{read_settings, Settings};
use crate::view::View;

/// The layout version of a `saved_answer` row this build writes and reads.
pub const SAVED_ANSWER_LAYOUT: i64 = 1;

/// The query language of a [`SavedQuery`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum QueryLanguage {
    /// SPARQL `SELECT` or `ASK`.
    Sparql,
    /// Read-only Cypher.
    Cypher,
}

impl QueryLanguage {
    /// The stored and JSON name: `"sparql"` or `"cypher"`.
    pub fn name(self) -> &'static str {
        match self {
            QueryLanguage::Sparql => "sparql",
            QueryLanguage::Cypher => "cypher",
        }
    }

    /// The language of a [`QueryLanguage::name`].
    pub fn from_name(name: &str) -> Option<QueryLanguage> {
        match name {
            "sparql" => Some(QueryLanguage::Sparql),
            "cypher" => Some(QueryLanguage::Cypher),
            _ => None,
        }
    }
}

/// What a saved answer re-evaluates: the query text, its parameters and the view,
/// stored exactly as given so a refresh never depends on current defaults.
///
/// ```
/// use tiramemsu::{CypherParams, CypherValue, QueryLanguage, SavedQuery, TimeRef, ViewSpec};
///
/// let q = SavedQuery::sparql("SELECT ?o WHERE { v:alice v:worksAt ?o }");
/// assert_eq!(q.language, QueryLanguage::Sparql);
/// assert_eq!(q.view, ViewSpec::NOW);
///
/// let mut params = CypherParams::new();
/// params.insert("name".into(), CypherValue::String("Alice".into()));
/// let q = SavedQuery::cypher("MATCH (p {name: $name}) RETURN p", params)
///     .on(ViewSpec::as_of(TimeRef::Tx(3)));
/// assert_eq!(q.view, ViewSpec::as_of(TimeRef::Tx(3)));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct SavedQuery {
    /// The language of `text`.
    pub language: QueryLanguage,
    /// The query text, verbatim.
    pub text: String,
    /// Cypher parameters (must be empty for SPARQL). Nodes, relationships and
    /// paths cannot be stored.
    pub params: CypherParams,
    /// The view the query runs on.
    pub view: ViewSpec,
}

impl SavedQuery {
    /// A SPARQL `SELECT` or `ASK` on the now view.
    pub fn sparql(text: impl Into<String>) -> SavedQuery {
        SavedQuery {
            language: QueryLanguage::Sparql,
            text: text.into(),
            params: CypherParams::new(),
            view: ViewSpec::NOW,
        }
    }

    /// A read-only Cypher query with parameters on the now view.
    pub fn cypher(text: impl Into<String>, params: CypherParams) -> SavedQuery {
        SavedQuery {
            language: QueryLanguage::Cypher,
            text: text.into(),
            params,
            view: ViewSpec::NOW,
        }
    }

    /// The same query on `view`.
    pub fn on(self, view: ViewSpec) -> SavedQuery {
        SavedQuery { view, ..self }
    }
}

/// The freshness of a saved answer. Only a successful refresh returns it to
/// `Fresh`; checking events can only move it towards `Stale`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AnswerStatus {
    /// No event since the checkpoint could have changed the result (as far as the
    /// cursor has processed).
    Fresh,
    /// An event or the clock may have changed the result; relevance cannot be
    /// excluded, so the answer must be re-evaluated before it is trusted.
    Recheck,
    /// A statement the result cited was retracted or superseded.
    Stale,
}

impl AnswerStatus {
    /// `"fresh"`, `"recheck"` or `"stale"`.
    pub fn name(self) -> &'static str {
        match self {
            AnswerStatus::Fresh => "fresh",
            AnswerStatus::Recheck => "recheck",
            AnswerStatus::Stale => "stale",
        }
    }

    fn code(self) -> i64 {
        self as i64
    }

    fn from_code(c: i64) -> AnswerStatus {
        match c {
            0 => AnswerStatus::Fresh,
            1 => AnswerStatus::Recheck,
            _ => AnswerStatus::Stale,
        }
    }
}

/// Why the cited statements of a saved answer do not prove its freshness. Every
/// reason is explicit; an answer with none is a fixed historical view whose
/// result depends only on immutable history.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CoverageReason {
    /// The view (or a pattern's own time scope) can still gain or lose
    /// statements: now, history, or an as-of point not yet in the past. Every
    /// later transaction requests a recheck.
    MutableView,
    /// The result cites no statements: Cypher, or SPARQL `ASK`.
    NoProvenance,
    /// `FILTER NOT EXISTS` or `MINUS`: an insertion can remove rows.
    NegativePattern,
    /// `FILTER EXISTS`: statements were tested but not cited.
    ExistsPattern,
    /// A recursive property path: its statements are not cited.
    RecursivePath,
    /// A virtual predicate (`sys:subject`, `tm:txAdded`, ...): computed from
    /// statement rows rather than matched as a statement.
    VirtualPredicate,
    /// Volatile values were read (now view only).
    Volatile,
    /// The result depends on the clock (`NOW()`, Cypher `datetime()`, ...), so
    /// it needs a recheck once the clock has moved, whatever the view.
    Clock,
}

impl CoverageReason {
    /// The stored and JSON name, e.g. `"mutableView"`.
    pub fn name(self) -> &'static str {
        match self {
            CoverageReason::MutableView => "mutableView",
            CoverageReason::NoProvenance => "noProvenance",
            CoverageReason::NegativePattern => "negativePattern",
            CoverageReason::ExistsPattern => "existsPattern",
            CoverageReason::RecursivePath => "recursivePath",
            CoverageReason::VirtualPredicate => "virtualPredicate",
            CoverageReason::Volatile => "volatile",
            CoverageReason::Clock => "clock",
        }
    }

    /// The reason of a [`CoverageReason::name`].
    pub fn from_name(name: &str) -> Option<CoverageReason> {
        Some(match name {
            "mutableView" => CoverageReason::MutableView,
            "noProvenance" => CoverageReason::NoProvenance,
            "negativePattern" => CoverageReason::NegativePattern,
            "existsPattern" => CoverageReason::ExistsPattern,
            "recursivePath" => CoverageReason::RecursivePath,
            "virtualPredicate" => CoverageReason::VirtualPredicate,
            "volatile" => CoverageReason::Volatile,
            "clock" => CoverageReason::Clock,
            _ => return None,
        })
    }
}

/// What moved a saved answer away from `Fresh`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum InvalidationCause {
    /// A cited statement was retracted (explicitly, by cascade, by a supersede
    /// or by a cardinality-one replacement). Makes the answer `Stale`.
    SupportRetracted,
    /// A statement was asserted under a mutable view. Makes it `Recheck`.
    Insertion,
    /// A statement the result did not cite was retracted under a mutable view.
    Retraction,
    /// A transaction without statement events (volatile values) under a mutable
    /// view.
    Transaction,
    /// The clock moved for a clock-sensitive query.
    Clock,
}

impl InvalidationCause {
    /// The JSON name, e.g. `"supportRetracted"`.
    pub fn name(self) -> &'static str {
        match self {
            InvalidationCause::SupportRetracted => "supportRetracted",
            InvalidationCause::Insertion => "insertion",
            InvalidationCause::Retraction => "retraction",
            InvalidationCause::Transaction => "transaction",
            InvalidationCause::Clock => "clock",
        }
    }

    fn code(self) -> i64 {
        match self {
            InvalidationCause::SupportRetracted => 0,
            InvalidationCause::Insertion => 1,
            InvalidationCause::Retraction => 2,
            InvalidationCause::Transaction => 3,
            InvalidationCause::Clock => 4,
        }
    }

    fn from_code(c: i64) -> Option<InvalidationCause> {
        Some(match c {
            0 => InvalidationCause::SupportRetracted,
            1 => InvalidationCause::Insertion,
            2 => InvalidationCause::Retraction,
            3 => InvalidationCause::Transaction,
            4 => InvalidationCause::Clock,
            _ => return None,
        })
    }
}

/// One logical invalidation: a saved answer moved to `status` because of
/// `cause`. [`Db::check_saved_answers`] reports each one exactly once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalidation {
    /// The saved answer.
    pub name: String,
    /// Its new status (`Recheck` or `Stale`).
    pub status: AnswerStatus,
    /// Why.
    pub cause: InvalidationCause,
    /// The transaction of the trigger (`None` for the clock).
    pub t: Option<TxId>,
    /// The triggering event (`None` for the clock and event-less transactions).
    pub event: Option<Event>,
}

/// A saved answer as stored: the query, the settings it was saved with, the last
/// successful result and what it cited, and its freshness bookkeeping.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedAnswer {
    /// The unique name.
    pub name: String,
    /// The query, parameters and view, as saved.
    pub query: SavedQuery,
    /// The `@vocab` IRI at save time (re-evaluation uses it, not the current one).
    pub vocab: Option<String>,
    /// The `(name, iri)` prefix table at save time.
    pub prefixes: Vec<(String, String)>,
    /// The last successful result as JSON: `{"vars", "rows", "provenance",
    /// "provenanceGaps"}` for a SPARQL `SELECT` (terms in the bindings' JSON
    /// form), `{"boolean"}` for `ASK`, `{"columns", "rows"}` for Cypher.
    pub result: J,
    /// The statements the result cited (SPARQL `SELECT` provenance), ascending.
    pub dependencies: Vec<Eid>,
    /// Why the dependencies do not prove freshness, ascending.
    pub coverage: Vec<CoverageReason>,
    /// The last transaction the result reflects.
    pub checkpoint: TxId,
    /// Events up to this transaction have been processed.
    pub cursor: TxId,
    /// Clock time (epoch ms) of the last successful evaluation.
    pub evaluated_at: i64,
    /// Number of successful evaluations (1 after saving).
    pub revision: u64,
    /// The freshness.
    pub status: AnswerStatus,
    /// What made it `Recheck` or `Stale` (`None` when `Fresh`).
    pub invalidation: Option<Invalidation>,
    /// The error of the last failed refresh, cleared by a successful one.
    pub error: Option<String>,
}

impl SavedAnswer {
    /// True when the status is `Fresh`.
    pub fn is_fresh(&self) -> bool {
        self.status == AnswerStatus::Fresh
    }

    /// The result of a SPARQL `SELECT` as [`Solutions`], with its provenance
    /// (`None` for `ASK` and Cypher).
    ///
    /// # Errors
    ///
    /// `InvalidTerm` when the stored JSON is malformed.
    pub fn solutions(&self) -> Result<Option<Solutions>> {
        if self.query.language != QueryLanguage::Sparql || self.result.get("vars").is_none() {
            return Ok(None);
        }
        solutions_from_json(&self.result).map(Some)
    }

    /// The answer of a SPARQL `ASK` (`None` otherwise).
    pub fn boolean(&self) -> Option<bool> {
        match self.query.language {
            QueryLanguage::Sparql => self.result.get("boolean").and_then(J::as_bool),
            QueryLanguage::Cypher => None,
        }
    }
}

// ---------------------------------------------------------------------------
// JSON forms

fn bad(reason: impl Into<String>) -> Error {
    Error::InvalidTerm {
        position: tm_core::Position::Value,
        reason: format!("saved answer: {}", reason.into()),
    }
}

const MAX_SAFE: u64 = 1 << 53;

/// The JSON form of a term, the one the bindings use (`bindings/json`).
pub(crate) fn term_json(v: &Value) -> J {
    match v {
        Value::Iri(s) => json!({ "iri": s }),
        Value::Node(n) => json!({ "node": n }),
        Value::BNode(n) => json!({ "bnode": n }),
        Value::Stmt(e) => json!({ "stmt": e.n() }),
        Value::Tx(t) => json!({ "tx": t.0 }),
        Value::Int(i) if i.unsigned_abs() <= MAX_SAFE => json!(i),
        Value::Int(i) => json!({ "$int": i.to_string() }),
        Value::Bool(b) => json!(b),
        Value::Str(s) => json!(s),
        Value::LangStr { lex, lang } => json!({ "lex": lex, "lang": lang }),
        Value::Double(x) if x.is_finite() && x.fract() != 0.0 => json!(x),
        _ => json!({ "lex": v.lexical(), "datatype": v.datatype() }),
    }
}

fn term_from_json(j: &J) -> Result<Value> {
    Ok(match j {
        J::String(s) => Value::Str(s.clone()),
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Value::Int(i),
            (None, Some(f)) => Value::Double(f),
            _ => return Err(bad(format!("number {n} is not a term"))),
        },
        J::Object(o) => {
            let text = |k: &str| o.get(k).and_then(J::as_str);
            let num = |k: &str| o.get(k).and_then(J::as_u64);
            if let Some(iri) = text("iri") {
                Value::iri(iri)
            } else if let Some(n) = num("node") {
                Value::Node(n)
            } else if let Some(n) = num("bnode") {
                Value::BNode(n)
            } else if let Some(n) = num("stmt") {
                Value::Stmt(Eid::new(n))
            } else if let Some(n) = num("tx") {
                Value::Tx(TxId(n))
            } else if let Some(d) = text("$int") {
                Value::big_integer(d)
            } else if let Some(lex) = text("lex") {
                Value::literal(lex, text("datatype"), text("lang"))
            } else {
                return Err(bad(format!("{j} is not a term")));
            }
        }
        _ => return Err(bad(format!("{j} is not a term"))),
    })
}

fn solutions_json(s: &Solutions) -> J {
    let rows: Vec<J> = s
        .rows
        .iter()
        .map(|r| {
            J::Array(
                r.iter()
                    .map(|c| c.as_ref().map_or(J::Null, term_json))
                    .collect(),
            )
        })
        .collect();
    let provenance = s.provenance.as_ref().map_or(J::Null, |p| {
        J::Array(
            p.iter()
                .map(|row| J::Array(row.iter().map(|e| json!(e.n())).collect()))
                .collect(),
        )
    });
    json!({
        "vars": s.vars,
        "rows": rows,
        "provenance": provenance,
        "provenanceGaps": s.provenance_gaps.iter().map(|g| g.name()).collect::<Vec<_>>(),
    })
}

fn solutions_from_json(j: &J) -> Result<Solutions> {
    let arr = |k: &str| {
        j.get(k)
            .and_then(J::as_array)
            .ok_or_else(|| bad(format!("result has no {k} array")))
    };
    let vars = arr("vars")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| bad("variable"))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut rows = Vec::new();
    for r in arr("rows")? {
        let cells = r.as_array().ok_or_else(|| bad("row"))?;
        rows.push(
            cells
                .iter()
                .map(|c| match c {
                    J::Null => Ok(None),
                    c => term_from_json(c).map(Some),
                })
                .collect::<Result<Vec<_>>>()?,
        );
    }
    let provenance = match j.get("provenance") {
        None | Some(J::Null) => None,
        Some(p) => Some(
            p.as_array()
                .ok_or_else(|| bad("provenance"))?
                .iter()
                .map(eids_from_json)
                .collect::<Result<Vec<_>>>()?,
        ),
    };
    let mut provenance_gaps = Vec::new();
    if let Some(gaps) = j.get("provenanceGaps").and_then(J::as_array) {
        for g in gaps {
            match g.as_str() {
                Some("recursivePath") => {
                    provenance_gaps.push(tm_sparql::results::ProvenanceGap::RecursivePath)
                }
                _ => return Err(bad(format!("unknown provenance gap {g}"))),
            }
        }
    }
    Ok(Solutions {
        vars,
        rows,
        provenance,
        provenance_gaps,
    })
}

fn eids_from_json(j: &J) -> Result<Vec<Eid>> {
    j.as_array()
        .ok_or_else(|| bad("eid list"))?
        .iter()
        .map(|e| e.as_u64().map(Eid::new).ok_or_else(|| bad("eid")))
        .collect()
}

/// The JSON view descriptor of the bindings: `{"kind", "tx" | "instant", "validAt"}`.
pub(crate) fn view_json(v: &ViewSpec) -> J {
    let mut o = Map::new();
    match v.tx {
        TxSel::Now => {
            o.insert("kind".into(), json!("now"));
        }
        TxSel::History => {
            o.insert("kind".into(), json!("history"));
        }
        TxSel::AsOf(TimeRef::Tx(t)) => {
            o.insert("kind".into(), json!("asOf"));
            o.insert("tx".into(), json!(t));
        }
        TxSel::AsOf(TimeRef::Instant(ms)) => {
            o.insert("kind".into(), json!("asOf"));
            o.insert("instant".into(), json!(ms));
        }
    }
    if let ValidSel::At(ms) = v.valid {
        o.insert("validAt".into(), json!(ms));
    }
    J::Object(o)
}

fn view_from_json(j: &J) -> Result<ViewSpec> {
    let int = |k: &str| j.get(k).and_then(J::as_i64);
    let tx = match j.get("kind").and_then(J::as_str) {
        Some("now") => TxSel::Now,
        Some("history") => TxSel::History,
        Some("asOf") => match (int("tx"), int("instant")) {
            (Some(t), _) => TxSel::AsOf(TimeRef::Tx(t as u64)),
            (None, Some(ms)) => TxSel::AsOf(TimeRef::Instant(ms)),
            _ => return Err(bad("asOf view without tx or instant")),
        },
        _ => return Err(bad(format!("{j} is not a view"))),
    };
    let valid = match int("validAt") {
        Some(ms) => ValidSel::At(ms),
        None => ValidSel::Unfiltered,
    };
    Ok(ViewSpec { tx, valid })
}

/// Lossless JSON of a Cypher parameter: tagged objects keep dates, big integers,
/// non-finite floats and maps apart from plain JSON.
fn param_json(v: &CypherValue) -> Result<J> {
    Ok(match v {
        CypherValue::Null => J::Null,
        CypherValue::Boolean(b) => json!(b),
        CypherValue::Integer(i) if i.unsigned_abs() <= MAX_SAFE => json!(i),
        CypherValue::Integer(i) => json!({ "$int": i.to_string() }),
        CypherValue::Float(x) if x.is_finite() => json!({ "$float": x }),
        CypherValue::Float(x) => json!({ "$float": x.to_string() }),
        CypherValue::String(s) => json!(s),
        CypherValue::Date(d) => json!({ "$date": d }),
        CypherValue::DateTime { ms, tz } => json!({ "$datetime": ms, "tz": tz }),
        CypherValue::LocalDateTime(ms) => json!({ "$localdatetime": ms }),
        CypherValue::List(l) => J::Array(l.iter().map(param_json).collect::<Result<_>>()?),
        CypherValue::Map(m) => json!({ "$map": params_json(m)? }),
        CypherValue::Node(_) | CypherValue::Relationship(_) | CypherValue::Path(_) => {
            return Err(Error::unsupported(
                "a node, relationship or path as a saved query parameter",
            ))
        }
    })
}

fn params_json(p: &CypherParams) -> Result<J> {
    Ok(J::Object(
        p.iter()
            .map(|(k, v)| Ok((k.clone(), param_json(v)?)))
            .collect::<Result<_>>()?,
    ))
}

fn param_from_json(j: &J) -> Result<CypherValue> {
    Ok(match j {
        J::Null => CypherValue::Null,
        J::Bool(b) => CypherValue::Boolean(*b),
        J::Number(n) => CypherValue::Integer(n.as_i64().ok_or_else(|| bad("parameter"))?),
        J::String(s) => CypherValue::String(s.clone()),
        J::Array(a) => CypherValue::List(a.iter().map(param_from_json).collect::<Result<_>>()?),
        J::Object(o) => {
            let int = |k: &str| o.get(k).and_then(J::as_i64);
            if let Some(d) = o.get("$int").and_then(J::as_str) {
                CypherValue::Integer(d.parse().map_err(|_| bad("parameter $int"))?)
            } else if let Some(f) = o.get("$float") {
                CypherValue::Float(match f {
                    J::String(s) => s.parse().map_err(|_| bad("parameter $float"))?,
                    f => f.as_f64().ok_or_else(|| bad("parameter $float"))?,
                })
            } else if let Some(d) = int("$date") {
                CypherValue::Date(d)
            } else if let Some(ms) = int("$datetime") {
                CypherValue::DateTime {
                    ms,
                    tz: int("tz").unwrap_or(0) as i16,
                }
            } else if let Some(ms) = int("$localdatetime") {
                CypherValue::LocalDateTime(ms)
            } else if let Some(m) = o.get("$map") {
                CypherValue::Map(params_from_json(m)?)
            } else {
                return Err(bad(format!("parameter {j}")));
            }
        }
    })
}

fn params_from_json(j: &J) -> Result<CypherParams> {
    j.as_object()
        .ok_or_else(|| bad("parameters"))?
        .iter()
        .map(|(k, v)| Ok((k.clone(), param_from_json(v)?)))
        .collect()
}

fn settings_json(s: &Settings) -> J {
    json!({
        "vocab": s.0,
        "prefixes": s.1.iter().map(|(n, i)| json!([n, i])).collect::<Vec<_>>(),
    })
}

fn settings_from_json(j: &J) -> Result<Settings> {
    let vocab = j.get("vocab").and_then(J::as_str).map(str::to_string);
    let mut prefixes = Vec::new();
    for p in j
        .get("prefixes")
        .and_then(J::as_array)
        .into_iter()
        .flatten()
    {
        match (p.get(0).and_then(J::as_str), p.get(1).and_then(J::as_str)) {
            (Some(n), Some(i)) => prefixes.push((n.to_string(), i.to_string())),
            _ => return Err(bad("prefix")),
        }
    }
    Ok((vocab, prefixes))
}

// ---------------------------------------------------------------------------
// Evaluation and coverage

/// What the IR of a query touches.
#[derive(Default)]
struct Analysis {
    negative: bool,
    exists: bool,
    recursive: bool,
    virtual_pred: bool,
    volatile: bool,
    views: Vec<tm_ir::View>,
}

impl Analysis {
    fn query(&mut self, q: &IrQuery) {
        self.op(&q.root);
    }

    fn pred(&mut self, p: &TermOrVar) {
        if let TermOrVar::Const(Value::Iri(iri)) = p {
            if VirtualPred::from_iri(iri).is_some() {
                self.virtual_pred = true;
            }
        }
    }

    fn op(&mut self, op: &tm_ir::Op) {
        use tm_ir::Op as O;
        match op {
            O::Triple(t) => {
                self.views.push(t.view);
                self.volatile |= t.include_volatile;
                self.pred(&t.p);
            }
            O::Path(p) => {
                self.views.push(p.view);
                self.recursive |= recursive(&p.path);
            }
            O::Text(t) => self.views.push(t.view),
            O::Values(_) | O::Join(_) | O::Union(_) | O::Project(_) => {}
            O::Unnest(u) => self.expr(&u.list),
            O::LeftJoin(l) => {
                if let Some(c) = &l.cond {
                    self.expr(c);
                }
            }
            O::Filter(f) => self.expr(&f.cond),
            O::Extend(e) => self.expr(&e.expr),
            O::Aggregate(a) => {
                for agg in &a.aggs {
                    if let Some(e) = &agg.arg {
                        self.expr(e);
                    }
                }
            }
            O::OrderLimit(o) => o.keys.iter().for_each(|k| self.expr(&k.expr)),
            O::RowNumber(r) => r.order.iter().for_each(|k| self.expr(&k.expr)),
        }
        for c in op.children() {
            self.op(c);
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Var(_) | Expr::Const(_) | Expr::Param(_) | Expr::Bound(_) => {}
            Expr::Cmp(_, a, b) | Expr::SameTerm(a, b) | Expr::Arith(_, a, b) => {
                self.expr(a);
                self.expr(b);
            }
            Expr::And(v) | Expr::Or(v) | Expr::Coalesce(v) | Expr::List(v) | Expr::Func(_, v) => {
                v.iter().for_each(|x| self.expr(x))
            }
            Expr::Not(a) | Expr::Neg(a) => self.expr(a),
            Expr::In(a, v, _) => {
                self.expr(a);
                v.iter().for_each(|x| self.expr(x));
            }
            Expr::If(a, b, c) => {
                self.expr(a);
                self.expr(b);
                self.expr(c);
            }
            Expr::Exists(op, negated) => {
                if *negated {
                    self.negative = true;
                } else {
                    self.exists = true;
                }
                self.op(op);
            }
            Expr::Lookup(l) => {
                self.views.push(l.view);
                self.volatile |= l.include_volatile;
                self.pred(&l.pred);
                self.expr(&l.subject);
            }
        }
    }
}

fn recursive(p: &PathExpr) -> bool {
    match p {
        PathExpr::Pred(_) => false,
        PathExpr::ZeroOrMore(_) | PathExpr::OneOrMore(_) => true,
        PathExpr::Repeat { max: None, .. } => true,
        PathExpr::Repeat { inner, .. } | PathExpr::Inverse(inner) | PathExpr::ZeroOrOne(inner) => {
            recursive(inner)
        }
        PathExpr::Seq(v) | PathExpr::Alt(v) => v.iter().any(recursive),
    }
}

/// The store's head when an evaluation starts: `last_t` and `last_instant`.
#[derive(Copy, Clone)]
struct Head {
    t: u64,
    instant: i64,
}

fn read_head(e: &mut dyn Executor) -> Result<Head> {
    Ok(Head {
        t: read::last_t(e)?,
        instant: e
            .query_i64("SELECT value FROM meta WHERE key = 'last_instant'", &[])?
            .unwrap_or(0),
    })
}

/// True when the transaction-time selection can no longer change: an as-of point
/// at or before the head. Instants of later transactions are strictly larger.
fn fixed(tx: TxSel, head: Head) -> bool {
    match tx {
        TxSel::Now | TxSel::History => false,
        TxSel::AsOf(TimeRef::Tx(t)) => t <= head.t,
        TxSel::AsOf(TimeRef::Instant(ms)) => ms <= head.instant,
    }
}

/// Cypher functions that read the clock when called without arguments; the
/// text is scanned conservatively (any call counts).
const CYPHER_CLOCK: [&str; 6] = [
    "datetime(",
    "localdatetime(",
    "date(",
    "timestamp(",
    "time(",
    "localtime(",
];

fn cypher_reads_clock(text: &str) -> bool {
    let lower: String = text
        .to_ascii_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    CYPHER_CLOCK.iter().any(|f| lower.contains(f))
}

/// One successful evaluation.
struct Evaluation {
    result: J,
    deps: Vec<Eid>,
    coverage: Vec<CoverageReason>,
}

fn evaluate(
    view: &View<'_>,
    q: &SavedQuery,
    settings: &Settings,
    head: Head,
) -> Result<Evaluation> {
    view.op(|| {
        let mut a = Analysis::default();
        let mut cov = Vec::new();
        let mut deps = Vec::new();
        let result = match q.language {
            QueryLanguage::Sparql => {
                if !q.params.is_empty() {
                    return Err(Error::unsupported("parameters for a saved SPARQL query"));
                }
                let env = view.sparql_env_with(Some(settings))?;
                let Prepared::Query(plan) = tm_sparql::prepare(&q.text, &env)? else {
                    return Err(Error::unsupported("a saved answer of an update"));
                };
                a.query(&plan.query);
                // NOW() is a constant of the lowered query: lowering again at
                // another instant shows whether the query reads the clock
                let mut later = env.clone();
                later.now_ms = env.now_ms.wrapping_add(1);
                if let Prepared::Query(p2) = tm_sparql::prepare(&q.text, &later)? {
                    if p2.query != plan.query {
                        cov.push(CoverageReason::Clock);
                    }
                }
                match &plan.form {
                    QueryForm::Select => {
                        let r = view.run_provenance(&plan)?;
                        let sol = r
                            .solutions()
                            .ok_or_else(|| Error::invalid_query("SELECT without solutions"))?;
                        for row in sol.provenance.iter().flatten() {
                            deps.extend(row.iter().copied());
                        }
                        if sol.provenance_complete() == Some(false) {
                            cov.push(CoverageReason::RecursivePath);
                        }
                        solutions_json(sol)
                    }
                    QueryForm::Ask => {
                        cov.push(CoverageReason::NoProvenance);
                        let b = view.run_plan(&plan)?;
                        json!({ "boolean": matches!(b, crate::SparqlResult::Boolean(true)) })
                    }
                    QueryForm::Construct { .. } => {
                        return Err(Error::unsupported("a saved answer of a CONSTRUCT query"))
                    }
                }
            }
            QueryLanguage::Cypher => {
                let mut trace = CypherTrace::default();
                let r = view.cypher_traced(&q.text, &q.params, settings, &mut trace)?;
                for ir in &trace.queries {
                    a.query(ir);
                }
                a.volatile |= trace.volatile;
                cov.push(CoverageReason::NoProvenance);
                if cypher_reads_clock(&q.text) {
                    cov.push(CoverageReason::Clock);
                }
                r.to_json()
            }
        };
        let flags = [
            (a.negative, CoverageReason::NegativePattern),
            (a.exists, CoverageReason::ExistsPattern),
            (a.recursive, CoverageReason::RecursivePath),
            (a.virtual_pred, CoverageReason::VirtualPredicate),
            (a.volatile, CoverageReason::Volatile),
        ];
        cov.extend(flags.iter().filter(|(on, _)| *on).map(|(_, r)| *r));
        let mutable = !fixed(q.view.tx, head) || a.views.iter().any(|v| !fixed(v.tx, head));
        if mutable {
            cov.push(CoverageReason::MutableView);
        }
        cov.sort();
        cov.dedup();
        deps.sort();
        deps.dedup();
        Ok(Evaluation {
            result,
            deps,
            coverage: cov,
        })
    })
}

// ---------------------------------------------------------------------------
// Rows

const COLUMNS: &str = "name, layout, language, query, params, view, settings, result, coverage, \
    checkpoint, cursor, evaluated_at, revision, status, cause, cause_t, cause_eid, cause_op, \
    cause_kind, error";

fn text_col(r: &[SqlValue], i: usize) -> Result<&str> {
    r[i].as_str().ok_or_else(|| bad("missing text column"))
}

fn json_col(r: &[SqlValue], i: usize) -> Result<J> {
    serde_json::from_str(text_col(r, i)?).map_err(|e| bad(e.to_string()))
}

fn int_col(r: &[SqlValue], i: usize) -> i64 {
    r[i].as_i64().unwrap_or(0)
}

fn eid_of(raw: i64) -> Eid {
    Eid::from_oid(ObjectId::from_raw(raw)).unwrap_or(Eid::new(0))
}

fn row_answer(e: &mut dyn Executor, r: &[SqlValue]) -> Result<SavedAnswer> {
    let layout = int_col(r, 1);
    if layout != SAVED_ANSWER_LAYOUT {
        return Err(Error::unsupported(format!(
            "saved answer layout {layout} (this build reads {SAVED_ANSWER_LAYOUT})"
        )));
    }
    let name = text_col(r, 0)?.to_string();
    let language = QueryLanguage::from_name(text_col(r, 2)?).ok_or_else(|| bad("language"))?;
    let (vocab, prefixes) = settings_from_json(&json_col(r, 6)?)?;
    let coverage = json_col(r, 8)?
        .as_array()
        .ok_or_else(|| bad("coverage"))?
        .iter()
        .map(|c| {
            c.as_str()
                .and_then(CoverageReason::from_name)
                .ok_or_else(|| bad(format!("coverage reason {c}")))
        })
        .collect::<Result<Vec<_>>>()?;
    let status = AnswerStatus::from_code(int_col(r, 13));
    let invalidation = match r[14].as_i64().and_then(InvalidationCause::from_code) {
        Some(cause) if status != AnswerStatus::Fresh => {
            let t = r[15].as_i64().map(|t| TxId(t as u64));
            let event = match (t, r[16].as_i64()) {
                (Some(t), Some(eid)) => Some(Event {
                    t,
                    eid: eid_of(eid),
                    op: if int_col(r, 17) == 1 {
                        Op::Retract
                    } else {
                        Op::Assert
                    },
                    kind: r[18].as_i64().and_then(RetKind::from_i64),
                }),
                _ => None,
            };
            Some(Invalidation {
                name: name.clone(),
                status,
                cause,
                t,
                event,
            })
        }
        _ => None,
    };
    let mut dependencies = Vec::new();
    e.query(
        "SELECT eid FROM saved_answer_dep WHERE name = ?1 ORDER BY eid",
        &[SqlValue::from(name.as_str())],
        &mut |d| {
            dependencies.push(eid_of(d[0].as_i64().unwrap_or(0)));
            Ok(())
        },
    )?;
    dependencies.sort();
    Ok(SavedAnswer {
        query: SavedQuery {
            language,
            text: text_col(r, 3)?.to_string(),
            params: params_from_json(&json_col(r, 4)?)?,
            view: view_from_json(&json_col(r, 5)?)?,
        },
        name,
        vocab,
        prefixes,
        result: json_col(r, 7)?,
        dependencies,
        coverage,
        checkpoint: TxId(int_col(r, 9) as u64),
        cursor: TxId(int_col(r, 10) as u64),
        evaluated_at: int_col(r, 11),
        revision: int_col(r, 12) as u64,
        status,
        invalidation,
        error: r[19].as_str().map(str::to_string),
    })
}

fn load(e: &mut dyn Executor, name: &str) -> Result<Option<SavedAnswer>> {
    let rows = e.rows(
        &format!("SELECT {COLUMNS} FROM saved_answer WHERE name = ?1"),
        &[SqlValue::from(name)],
    )?;
    match rows.first() {
        Some(r) => row_answer(e, r).map(Some),
        None => Ok(None),
    }
}

fn load_all(e: &mut dyn Executor) -> Result<Vec<SavedAnswer>> {
    let rows = e.rows(
        &format!("SELECT {COLUMNS} FROM saved_answer ORDER BY name"),
        &[],
    )?;
    rows.iter().map(|r| row_answer(e, r)).collect()
}

fn not_found(name: &str) -> Error {
    Error::SavedAnswerNotFound {
        name: name.to_string(),
    }
}

/// Writes a successful evaluation: result, dependencies, coverage, checkpoint and
/// cursor at `head`, status fresh, error cleared.
#[allow(clippy::too_many_arguments)]
fn store_evaluation(
    e: &mut dyn Executor,
    name: &str,
    q: &SavedQuery,
    settings: &Settings,
    ev: &Evaluation,
    head: Head,
    now_ms: i64,
    revision: u64,
) -> Result<()> {
    let coverage: Vec<&str> = ev.coverage.iter().map(|c| c.name()).collect();
    e.execute(
        "INSERT OR REPLACE INTO saved_answer(name, layout, language, query, params, view, \
         settings, result, coverage, checkpoint, cursor, evaluated_at, revision, status, \
         cause, cause_t, cause_eid, cause_op, cause_kind, error) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?11, ?12, 0, \
         NULL, NULL, NULL, NULL, NULL, NULL)",
        &[
            SqlValue::from(name),
            SqlValue::Integer(SAVED_ANSWER_LAYOUT),
            SqlValue::from(q.language.name()),
            SqlValue::from(q.text.as_str()),
            SqlValue::from(params_json(&q.params)?.to_string()),
            SqlValue::from(view_json(&q.view).to_string()),
            SqlValue::from(settings_json(settings).to_string()),
            SqlValue::from(ev.result.to_string()),
            SqlValue::from(json!(coverage).to_string()),
            SqlValue::Integer(head.t as i64),
            SqlValue::Integer(now_ms),
            SqlValue::Integer(revision as i64),
        ],
    )?;
    e.execute(
        "DELETE FROM saved_answer_dep WHERE name = ?1",
        &[SqlValue::from(name)],
    )?;
    for d in &ev.deps {
        e.execute(
            "INSERT INTO saved_answer_dep(name, eid) VALUES (?1, ?2)",
            &[SqlValue::from(name), SqlValue::Integer(d.oid().raw())],
        )?;
    }
    Ok(())
}

fn event_of(r: &[SqlValue]) -> Event {
    Event {
        t: TxId(int_col(r, 0) as u64),
        eid: eid_of(int_col(r, 1)),
        op: if r[2].as_str() == Some("retract") {
            Op::Retract
        } else {
            Op::Assert
        },
        kind: r[3].as_i64().and_then(RetKind::from_i64),
    }
}

/// Processes the events after `a.cursor` up to `head` for one answer and records
/// the result. Deterministic in (status, cursor, log): replaying a range that was
/// not committed gives the same marks, and a processed range is never seen
/// again, so every logical invalidation is reported once.
fn process(
    e: &mut dyn Executor,
    a: &SavedAnswer,
    head: u64,
    now_ms: i64,
) -> Result<Option<Invalidation>> {
    let cursor = a.cursor.0;
    let mutable = a.coverage.contains(&CoverageReason::MutableView);
    let range = [
        SqlValue::Integer(cursor as i64),
        SqlValue::Integer(head as i64),
    ];
    let mut found: Option<Invalidation> = None;
    let mark = |status, cause, t: Option<TxId>, event| Invalidation {
        name: a.name.clone(),
        status,
        cause,
        t,
        event,
    };
    if mutable && head > cursor {
        if a.status < AnswerStatus::Stale {
            let rows = e.rows(
                "SELECT t_ret, eid, 'retract', ret_kind FROM triple \
                 WHERE eid IN (SELECT eid FROM saved_answer_dep WHERE name = ?3) \
                 AND t_ret > ?1 AND t_ret <= ?2 ORDER BY t_ret, eid LIMIT 1",
                &[
                    range[0].clone(),
                    range[1].clone(),
                    SqlValue::from(a.name.as_str()),
                ],
            )?;
            if let Some(r) = rows.first() {
                let ev = event_of(r);
                found = Some(mark(
                    AnswerStatus::Stale,
                    InvalidationCause::SupportRetracted,
                    Some(ev.t),
                    Some(ev),
                ));
            }
        }
        if found.is_none() && a.status == AnswerStatus::Fresh {
            let rows = e.rows(
                "SELECT t, eid, op, kind FROM event WHERE t > ?1 AND t <= ?2 \
                 ORDER BY t, op, eid LIMIT 1",
                &range,
            )?;
            found = Some(match rows.first() {
                Some(r) => {
                    let ev = event_of(r);
                    let cause = match ev.op {
                        Op::Assert => InvalidationCause::Insertion,
                        Op::Retract => InvalidationCause::Retraction,
                    };
                    mark(AnswerStatus::Recheck, cause, Some(ev.t), Some(ev))
                }
                None => {
                    let t = e
                        .query_i64("SELECT min(t) FROM tx WHERE t > ?1 AND t <= ?2", &range)?
                        .map_or(TxId(cursor + 1), |t| TxId(t as u64));
                    mark(
                        AnswerStatus::Recheck,
                        InvalidationCause::Transaction,
                        Some(t),
                        None,
                    )
                }
            });
        }
    }
    if found.is_none()
        && a.status == AnswerStatus::Fresh
        && a.coverage.contains(&CoverageReason::Clock)
        && now_ms > a.evaluated_at
    {
        found = Some(mark(
            AnswerStatus::Recheck,
            InvalidationCause::Clock,
            None,
            None,
        ));
    }
    let new_cursor = cursor.max(head) as i64;
    match &found {
        Some(inv) => {
            let ev = inv.event;
            e.execute(
                "UPDATE saved_answer SET cursor = ?2, status = ?3, cause = ?4, cause_t = ?5, \
                 cause_eid = ?6, cause_op = ?7, cause_kind = ?8 WHERE name = ?1",
                &[
                    SqlValue::from(a.name.as_str()),
                    SqlValue::Integer(new_cursor),
                    SqlValue::Integer(inv.status.code()),
                    SqlValue::Integer(inv.cause.code()),
                    SqlValue::from(inv.t.map(|t| t.0 as i64)),
                    SqlValue::from(ev.map(|ev| ev.eid.oid().raw())),
                    SqlValue::from(ev.map(|ev| (ev.op == Op::Retract) as i64)),
                    SqlValue::from(ev.and_then(|ev| ev.kind).map(|k| k as i64)),
                ],
            )?;
        }
        None if new_cursor != cursor as i64 => {
            e.execute(
                "UPDATE saved_answer SET cursor = ?2 WHERE name = ?1",
                &[
                    SqlValue::from(a.name.as_str()),
                    SqlValue::Integer(new_cursor),
                ],
            )?;
        }
        None => {}
    }
    Ok(found)
}

impl Db {
    fn saved_view(&self, spec: ViewSpec) -> View<'_> {
        View::on_db(self, spec)
    }

    fn eval_saved<'b>(
        &'b self,
        q: &SavedQuery,
        settings: &Settings,
        budget: Option<&'b QueryBudget>,
    ) -> Result<(Evaluation, Head)> {
        crate::budget::run(budget, || {
            let head = self.read_committed(read_head)?;
            let view = self.saved_view(q.view);
            let ev = match budget {
                Some(b) => evaluate(&view.with_budget(b), q, settings, head)?,
                None => evaluate(&view, q, settings, head)?,
            };
            Ok((ev, head))
        })
    }

    /// Runs `query` and saves its answer under `name`, replacing an answer of
    /// that name: the query text, parameters and view exactly as given, the
    /// database `@vocab` and prefix table of this moment (a refresh reuses them),
    /// the result, the statements it cited and why they do not prove freshness
    /// ([`SavedAnswer::coverage`]). The checkpoint and the event cursor are the
    /// last transaction committed when the evaluation started. Saving writes a
    /// derived record: no transaction number is used and no event is logged.
    ///
    /// SPARQL `SELECT` runs with provenance, so its rows' eids become the
    /// dependencies; `ASK` and Cypher cite none (`NoProvenance`).
    ///
    /// # Errors
    ///
    /// The errors of the query; `Unsupported` for a SPARQL update or `CONSTRUCT`,
    /// SPARQL with parameters, a Cypher write clause, or a node, relationship or
    /// path parameter; `InvalidQuery` for an empty name; `ImportInProgress`
    /// while a bulk import holds the write lease; `Reentrant` inside a
    /// transaction.
    ///
    /// ```
    /// # use tiramemsu::*;
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let db = Db::open(dir.path().join("m.db"), OpenOptions::default())?;
    /// db.now().sparql("INSERT DATA { v:alice v:worksAt v:acme }")?;
    /// let a = db.save_answer("employer", &SavedQuery::sparql("SELECT ?o WHERE { v:alice v:worksAt ?o }"))?;
    /// assert!(a.is_fresh());
    /// assert_eq!(a.dependencies.len(), 1);
    /// assert_eq!(a.coverage, [CoverageReason::MutableView]);
    ///
    /// // superseding the cited statement makes it stale, with the event
    /// db.now().sparql("DELETE DATA { v:alice v:worksAt v:acme } ; INSERT DATA { v:alice v:worksAt v:globex }")?;
    /// let marks = db.check_saved_answers()?;
    /// assert_eq!(marks[0].status, AnswerStatus::Stale);
    /// assert_eq!(marks[0].cause, InvalidationCause::SupportRetracted);
    /// assert!(db.check_saved_answers()?.is_empty()); // reported once
    ///
    /// // only a successful re-run makes it fresh again
    /// let a = db.refresh_answer("employer")?;
    /// assert!(a.is_fresh());
    /// assert_eq!(a.revision, 2);
    /// # Ok::<(), Error>(())
    /// ```
    // @lat: [[query#Saved Answers]]
    pub fn save_answer(&self, name: &str, query: &SavedQuery) -> Result<SavedAnswer> {
        self.save_answer_with(name, query, None)
    }

    /// [`Db::save_answer`] with the evaluation bounded by `budget` (one operation).
    ///
    /// # Errors
    ///
    /// Those of [`Db::save_answer`], plus `Cancelled`, `DeadlineExceeded`,
    /// `PoolTimeout` and `ResultLimitExceeded`; nothing is saved after any error.
    pub fn save_answer_with(
        &self,
        name: &str,
        query: &SavedQuery,
        budget: Option<&QueryBudget>,
    ) -> Result<SavedAnswer> {
        if name.is_empty() {
            return Err(Error::invalid_query("a saved answer needs a name"));
        }
        if query.language == QueryLanguage::Sparql && !query.params.is_empty() {
            return Err(Error::unsupported("parameters for a saved SPARQL query"));
        }
        params_json(&query.params)?;
        let settings = self.read_committed(|e| read_settings(e))?;
        let (ev, head) = self.eval_saved(query, &settings, budget)?;
        let now = self.now_ms();
        self.derived_write(|e| {
            store_evaluation(e, name, query, &settings, &ev, head, now, 1)?;
            load(e, name)?.ok_or_else(|| not_found(name))
        })
    }

    /// The saved answer `name` as stored (`None` when there is none). Reading
    /// does not process events: call [`Db::check_saved_answers`] first for
    /// current marks.
    ///
    /// # Errors
    ///
    /// `Sqlite` on a read failure, `Unsupported` for a record of a newer layout.
    pub fn saved_answer(&self, name: &str) -> Result<Option<SavedAnswer>> {
        self.read_committed(|e| load(e, name))
    }

    /// Every saved answer, by name.
    ///
    /// # Errors
    ///
    /// As [`Db::saved_answer`].
    pub fn saved_answers(&self) -> Result<Vec<SavedAnswer>> {
        self.read_committed(load_all)
    }

    /// Deletes the saved answer `name`; returns whether there was one. Graph
    /// history is untouched.
    ///
    /// # Errors
    ///
    /// `ImportInProgress`, `Reentrant`, or `Sqlite`.
    pub fn delete_saved_answer(&self, name: &str) -> Result<bool> {
        self.derived_write(|e| {
            e.execute(
                "DELETE FROM saved_answer_dep WHERE name = ?1",
                &[SqlValue::from(name)],
            )?;
            Ok(e.execute(
                "DELETE FROM saved_answer WHERE name = ?1",
                &[SqlValue::from(name)],
            )? > 0)
        })
    }

    /// Processes the event log after each saved answer's cursor, up to the last
    /// committed transaction, and returns the invalidations it recorded, one per
    /// answer whose status changed. Conservative rules:
    ///
    /// - a retraction of a cited statement (explicit, cascade, supersede or
    ///   cardinality) under a mutable view makes the answer `Stale`, with that
    ///   event;
    /// - otherwise any later transaction under a mutable view makes a fresh
    ///   answer `Recheck`: an insertion may add a row, satisfy a `NOT EXISTS` or
    ///   fill an `OPTIONAL`, and relevance is not analysed;
    /// - a fixed historical view stays fresh, except that a clock-sensitive
    ///   query becomes `Recheck` once the clock has moved.
    ///
    /// The cursor and the marks are written in one transaction, so processing is
    /// idempotent and restartable: running it again, or after reopening, reports
    /// nothing twice. Nothing here clears a mark; only [`Db::refresh_answer`] does.
    ///
    /// # Errors
    ///
    /// `ImportInProgress`, `Reentrant`, or `Sqlite`.
    pub fn check_saved_answers(&self) -> Result<Vec<Invalidation>> {
        let now = self.now_ms();
        self.derived_write(|e| {
            let head = read::last_t(e)?;
            let mut out = Vec::new();
            for a in load_all(e)? {
                if let Some(inv) = process(e, &a, head, now)? {
                    out.push(inv);
                }
            }
            Ok(out)
        })
    }

    /// Re-runs the saved answer `name` with its stored query, parameters, view
    /// and settings. On success the result, dependencies and coverage are
    /// replaced, the checkpoint and cursor advance to the transaction committed
    /// when the run started, the status is `Fresh` and the revision grows by one.
    ///
    /// # Errors
    ///
    /// `SavedAnswerNotFound`, and any error of the run. After a failed run the
    /// pending events are processed, the error is recorded in
    /// [`SavedAnswer::error`], and the old result, status and checkpoint stay:
    /// a stale answer stays stale.
    pub fn refresh_answer(&self, name: &str) -> Result<SavedAnswer> {
        self.refresh_answer_with(name, None)
    }

    /// [`Db::refresh_answer`] with the re-run bounded by `budget`.
    ///
    /// # Errors
    ///
    /// Those of [`Db::refresh_answer`], plus `Cancelled`, `DeadlineExceeded`,
    /// `PoolTimeout` and `ResultLimitExceeded`, each a failed refresh.
    pub fn refresh_answer_with(
        &self,
        name: &str,
        budget: Option<&QueryBudget>,
    ) -> Result<SavedAnswer> {
        let old = self.saved_answer(name)?.ok_or_else(|| not_found(name))?;
        let settings = (old.vocab.clone(), old.prefixes.clone());
        let now = self.now_ms();
        match self.eval_saved(&old.query, &settings, budget) {
            Ok((ev, head)) => self.derived_write(|e| {
                let cur = load(e, name)?.ok_or_else(|| not_found(name))?;
                if cur.query != old.query || (cur.vocab.clone(), cur.prefixes.clone()) != settings {
                    // saved anew while this refresh ran: the new record wins
                    return Ok(cur);
                }
                store_evaluation(
                    e,
                    name,
                    &cur.query,
                    &settings,
                    &ev,
                    head,
                    now,
                    cur.revision + 1,
                )?;
                load(e, name)?.ok_or_else(|| not_found(name))
            }),
            Err(err) => {
                let message = err.to_string();
                self.derived_write(|e| {
                    let head = read::last_t(e)?;
                    if let Some(cur) = load(e, name)? {
                        process(e, &cur, head, now)?;
                        e.execute(
                            "UPDATE saved_answer SET error = ?2 WHERE name = ?1",
                            &[SqlValue::from(name), SqlValue::from(message.as_str())],
                        )?;
                    }
                    Ok(())
                })?;
                Err(err)
            }
        }
    }
}
