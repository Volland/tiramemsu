//! The program interpreter: clause by clause over rows of Cypher values. Graph
//! patterns and lookups go through the IR ([`crate::runner::Runner`]); expressions,
//! projection and aggregation are evaluated here (see `PARSER.md` and the task notes
//! for the reason).

use std::collections::{BTreeMap, HashMap, HashSet};

use tm_ir::{TimeRef, View};

use crate::ast::*;
use crate::error::{CResult, CypherError};
use crate::program::{CypherParams, CypherProgram};
use crate::runner::Runner;
use crate::value::{CypherResult, CypherValue, Val};
use crate::vocab::Vocab;

pub mod access;
pub mod agg;
pub mod eval;
pub mod funcs;
pub mod pattern;
pub mod project;
pub mod text;
pub mod time;
pub mod write;
pub mod write_set;

/// A row: variable name to value.
pub type Row = BTreeMap<String, Val>;

/// Predicate classification flags of one view (`sys:isEdge`).
#[derive(Clone, Debug, Default)]
pub struct Flags {
    /// Predicates with `sys:isEdge true`.
    pub edge_true: HashSet<String>,
    /// Predicates with `sys:isEdge false`.
    pub edge_false: HashSet<String>,
}

/// One execution of a program.
pub struct Exec<'r> {
    pub(crate) runner: &'r mut dyn Runner,
    pub(crate) vocab: Vocab,
    pub(crate) params: BTreeMap<String, Val>,
    pub(crate) now_ms: i64,
    pub(crate) views: Vec<View>,
    pub(crate) flags: HashMap<View, Flags>,
    pub(crate) props: HashMap<(String, String, View), Vec<Val>>,
    pub(crate) writer: write::WriteState,
    pub(crate) uid: usize,
}

/// Converts a supplied parameter to a value.
pub fn param_val(v: &CypherValue) -> Val {
    match v {
        CypherValue::Null => Val::Null,
        CypherValue::Boolean(b) => Val::Bool(*b),
        CypherValue::Integer(i) => Val::Int(*i),
        CypherValue::Float(x) => Val::Float(*x),
        CypherValue::String(s) => Val::Str(s.clone()),
        CypherValue::Date(d) => Val::Date(*d),
        CypherValue::DateTime { ms, tz } => Val::DateTime { ms: *ms, tz: *tz },
        CypherValue::LocalDateTime(ms) => Val::LocalDateTime(*ms),
        CypherValue::List(l) => Val::List(l.iter().map(param_val).collect()),
        CypherValue::Map(m) => Val::Map(m.iter().map(|(k, v)| (k.clone(), param_val(v))).collect()),
        CypherValue::Node(n) => Val::Node(n.term.clone()),
        CypherValue::Relationship(r) => Val::Rel(r.eid),
        CypherValue::Path(p) => {
            let mut out = Vec::new();
            for (i, n) in p.nodes.iter().enumerate() {
                out.push(Val::Node(n.term.clone()));
                if let Some(r) = p.rels.get(i) {
                    out.push(Val::Rel(r.eid));
                }
            }
            Val::Path(out)
        }
    }
}

/// Runs a compiled program on `runner` and returns the result table.
///
/// Clauses run in order over rows of values; every write clause goes through
/// [`Runner::with_tx`], so the whole query is one transaction when the runner
/// wraps one. A query without `RETURN` returns no rows.
///
/// # Errors
///
/// [`CypherError::Eval`] for runtime type errors, division by zero and values
/// that cannot be stored, [`CypherError::Unsupported`] for a write on a
/// read-only runner, and [`CypherError::Core`] for store failures.
pub fn run(
    prog: &CypherProgram,
    params: &CypherParams,
    runner: &mut dyn Runner,
) -> CResult<CypherResult> {
    let now_ms = runner.now_ms();
    let mut ex = Exec {
        runner,
        vocab: prog.ctx.vocab.clone(),
        params: params
            .iter()
            .map(|(k, v)| (k.clone(), param_val(v)))
            .collect(),
        now_ms,
        views: vec![prog.ctx.view],
        flags: HashMap::new(),
        props: HashMap::new(),
        writer: write::WriteState::default(),
        uid: 0,
    };
    let (cols, mut rows) = ex.run_query(&prog.query, vec![Row::new()], true)?;
    ex.finish_writes()?;
    if cols.is_empty() {
        // a query without RETURN produces no rows
        rows.clear();
    }
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let mut line = Vec::with_capacity(cols.len());
        for c in &cols {
            let v = r.get(c).cloned().unwrap_or(Val::Null);
            line.push(ex.materialize(&v)?);
        }
        out.push(line);
    }
    Ok(CypherResult {
        columns: cols,
        rows: out,
        report: None,
    })
}

impl Exec<'_> {
    pub(crate) fn view(&self) -> View {
        *self.views.last().unwrap_or(&View::NOW)
    }

    pub(crate) fn next_uid(&mut self) -> usize {
        self.uid += 1;
        self.uid
    }

    /// Runs a (possibly `UNION`) query over `input` rows. Returns column names and rows.
    pub(crate) fn run_query(
        &mut self,
        q: &Query,
        input: Vec<Row>,
        top: bool,
    ) -> CResult<(Vec<String>, Vec<Row>)> {
        let _ = top;
        let mut cols: Vec<String> = Vec::new();
        let mut rows: Vec<Row> = Vec::new();
        for (i, part) in q.parts.iter().enumerate() {
            let (c, r) = self.run_single(part, input.clone())?;
            if i == 0 {
                cols = c;
                rows = r;
            } else {
                let all = q.unions[i - 1];
                rows.extend(r);
                if !all {
                    rows = dedupe(rows, &cols);
                }
            }
        }
        // `UNION ALL` mixed with `UNION`: any plain UNION dedupes everything so far.
        Ok((cols, rows))
    }

    /// Runs one branch.
    pub(crate) fn run_single(
        &mut self,
        s: &SingleQuery,
        input: Vec<Row>,
    ) -> CResult<(Vec<String>, Vec<Row>)> {
        let pushed = match &s.time {
            Some(t) => {
                let v = self.resolve_time(t)?;
                self.views.push(v);
                true
            }
            None => false,
        };
        let r = self.run_clauses(&s.clauses, input);
        if pushed {
            self.views.pop();
        }
        r
    }

    fn run_clauses(
        &mut self,
        clauses: &[Clause],
        mut rows: Vec<Row>,
    ) -> CResult<(Vec<String>, Vec<Row>)> {
        let mut cols: Vec<String> = Vec::new();
        let mut scope: Vec<String> = match rows.first() {
            Some(r) => r.keys().cloned().collect(),
            None => Vec::new(),
        };
        for c in clauses {
            match c {
                Clause::Match {
                    optional,
                    mode,
                    pattern,
                    where_,
                    ..
                } => {
                    add_pattern_names(&mut scope, pattern);
                    rows = self.exec_match(rows, pattern, where_.as_ref(), *optional, *mode)?;
                }
                Clause::Unwind { expr, var } => {
                    add_name(&mut scope, &var.text);
                    let mut out = Vec::new();
                    for r in rows {
                        match self.eval(expr, &r)? {
                            Val::Null => {}
                            Val::List(l) => {
                                for x in l {
                                    let mut nr = r.clone();
                                    nr.insert(var.text.clone(), x);
                                    out.push(nr);
                                }
                            }
                            other => {
                                let mut nr = r.clone();
                                nr.insert(var.text.clone(), other);
                                out.push(nr);
                            }
                        }
                    }
                    rows = out;
                }
                Clause::With { proj, where_ } => {
                    let (r, c) = self.project(rows, proj, where_.as_ref(), &sorted(&scope))?;
                    rows = r;
                    scope = c;
                }
                Clause::Return(proj) => {
                    let (r, c) = self.project(rows, proj, None, &sorted(&scope))?;
                    rows = r;
                    cols = c;
                }
                Clause::Subquery { body, imports, .. } => {
                    for c in subquery_cols(body) {
                        add_name(&mut scope, &c);
                    }
                    rows = self.exec_subquery(rows, body, imports)?;
                }
                Clause::Procedure {
                    name, args, yields, ..
                } => {
                    let standalone = clauses.len() == 1;
                    for (c, a) in yields {
                        add_name(&mut scope, &a.as_ref().unwrap_or(c).text);
                    }
                    let (r, c) = self.exec_procedure(rows, name, args, yields, standalone)?;
                    rows = r;
                    if standalone {
                        cols = c;
                    }
                }
                Clause::Create { .. }
                | Clause::Merge { .. }
                | Clause::Set(_)
                | Clause::Remove(_)
                | Clause::Delete { .. } => {
                    match c {
                        Clause::Create { pattern, .. } => add_pattern_names(&mut scope, pattern),
                        Clause::Merge { part, .. } => {
                            add_pattern_names(&mut scope, std::slice::from_ref(part))
                        }
                        _ => {}
                    }
                    rows = self.exec_write(rows, c)?;
                }
            }
        }
        Ok((cols, rows))
    }

    fn exec_subquery(
        &mut self,
        rows: Vec<Row>,
        body: &Query,
        imports: &[Name],
    ) -> CResult<Vec<Row>> {
        let mut out = Vec::new();
        let mut memo: HashMap<String, (Vec<String>, Vec<Row>)> = HashMap::new();
        let writes = crate::sema::check::first_write(body).is_some();
        for r in rows {
            let mut seed = Row::new();
            for i in imports {
                seed.insert(i.text.clone(), r.get(&i.text).cloned().unwrap_or(Val::Null));
            }
            let key = imports
                .iter()
                .map(|i| crate::value::group_key(seed.get(&i.text).unwrap_or(&Val::Null)))
                .collect::<Vec<_>>()
                .join("|");
            let (cols, res) = match memo.get(&key) {
                Some(x) if !writes => x.clone(),
                _ => {
                    let x = self.run_query(body, vec![seed], false)?;
                    if !writes {
                        memo.insert(key, x.clone());
                    }
                    x
                }
            };
            if cols.is_empty() {
                // a unit subquery keeps the outer row
                out.push(r);
                continue;
            }
            for sub in res {
                let mut nr = r.clone();
                for c in &cols {
                    nr.insert(c.clone(), sub.get(c).cloned().unwrap_or(Val::Null));
                }
                out.push(nr);
            }
        }
        Ok(out)
    }

    fn exec_procedure(
        &mut self,
        rows: Vec<Row>,
        name: &str,
        args: &[Expr],
        yields: &[(Name, Option<Name>)],
        standalone: bool,
    ) -> CResult<(Vec<Row>, Vec<String>)> {
        let lname = name.to_ascii_lowercase();
        if lname == text::NAME {
            return self.exec_text_search(rows, args, yields, standalone);
        }
        let (col, values) = self.builtin_procedure(&lname)?;
        let mut out = Vec::new();
        let outname = |c: &str| -> Option<String> {
            if yields.is_empty() {
                standalone.then(|| c.to_string())
            } else {
                yields
                    .iter()
                    .find(|(n, _)| n.text == c)
                    .map(|(n, a)| a.as_ref().unwrap_or(n).text.clone())
            }
        };
        let target = outname(col);
        for r in rows {
            for v in &values {
                let mut nr = r.clone();
                if let Some(t) = &target {
                    nr.insert(t.clone(), v.clone());
                }
                out.push(nr);
            }
        }
        Ok((out, vec![col.to_string()]))
    }

    /// Resolves a `USE` clause against the current view.
    pub(crate) fn resolve_time(&mut self, t: &TimeSel) -> CResult<View> {
        time::resolve(self, t)
    }
}

/// Removes duplicate rows by Cypher equivalence over `cols`.
pub fn dedupe(rows: Vec<Row>, cols: &[String]) -> Vec<Row> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for r in rows {
        let key: Vec<String> = cols
            .iter()
            .map(|c| crate::value::group_key(r.get(c).unwrap_or(&Val::Null)))
            .collect();
        if seen.insert(key.join("\u{1}")) {
            out.push(r);
        }
    }
    out
}

pub(crate) fn core<E: Into<tm_core::Error>>(e: E) -> CypherError {
    CypherError::Core(e.into())
}

/// Helper for tests and docs.
pub fn asof(t: u64) -> TimeRef {
    TimeRef::Tx(t)
}

fn add_name(scope: &mut Vec<String>, n: &str) {
    if !scope.iter().any(|x| x == n) {
        scope.push(n.to_string());
    }
}

fn sorted(scope: &[String]) -> Vec<String> {
    let mut v = scope.to_vec();
    v.sort();
    v
}

fn add_pattern_names(scope: &mut Vec<String>, p: &[PatternPart]) {
    for part in p {
        for n in &part.nodes {
            if let Some(v) = &n.var {
                add_name(scope, &v.text);
            }
        }
        for r in &part.rels {
            if let Some(v) = &r.var {
                add_name(scope, &v.text);
            }
        }
        if let Some(b) = &part.binding {
            add_name(scope, &b.text);
        }
    }
}

/// The result column names of a subquery body (its last branch's `RETURN`).
fn subquery_cols(q: &Query) -> Vec<String> {
    let Some(last) = q.parts.first() else {
        return Vec::new();
    };
    for c in last.clauses.iter().rev() {
        if let Clause::Return(p) = c {
            let mut v: Vec<String> = Vec::new();
            for it in &p.items {
                v.push(match &it.alias {
                    Some(a) => a.text.clone(),
                    None => it.text.clone(),
                });
            }
            return v;
        }
    }
    Vec::new()
}
