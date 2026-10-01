//! Fact bundles: a self-contained, portable set of statements around a root
//! statement (spec `fact-bundles`).
//!
//! A bundle moves a fact with its layers and its evidence from one database file to
//! another. Eids, node ids and transaction numbers are local to a file, so a bundle
//! names its statements by bundle-local ids and its anonymous nodes by bundle-local
//! labels; everything else travels as a [`Value`]. [`crate::read::bundle`] builds one
//! from a view and [`crate::Tx::import_bundle`] writes one into a transaction. The
//! JSON and N-Triples forms live in the facade crate (`tiramemsu::BundleFormat`), so
//! this crate keeps its dependency list.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap, HashSet};

use crate::engine::reserved;
use crate::error::{Error, Position, Result};
use crate::exec::{Executor, Params};
use crate::id::{Eid, ObjectId, Tag};
use crate::read::{dependents, visible};
use crate::report::Valid;
use crate::term::TermReader;
use crate::value::Value;
use crate::view::{scan_predicates, ViewSpec};
use crate::vocab;

/// A subject or object position of a [`BundleStatement`].
#[derive(Clone, Debug, PartialEq)]
pub enum BTerm {
    /// An IRI or a literal, encoded in the target's own dictionary on import.
    Value(Value),
    /// Another statement of the same bundle, by its bundle-local id.
    Stmt(u32),
    /// An anonymous node (`NODE` or `BNODE` in the source) by its bundle-local
    /// label; import mints one fresh node per label.
    Node(u32),
}

/// One statement of a [`Bundle`].
#[derive(Clone, Debug, PartialEq)]
pub struct BundleStatement {
    /// The bundle-local id (the statement's position in a bundle built by
    /// [`crate::read::bundle`]).
    pub local: u32,
    /// Subject.
    pub s: BTerm,
    /// Predicate: an IRI.
    pub p: Value,
    /// Object.
    pub o: BTerm,
    /// Valid time, carried as is. Transaction time is the importing transaction's.
    pub valid: Valid,
}

/// A portable set of statements around a root statement: the statements that stand
/// on the root in a view, plus the statements they reference, transitively.
///
/// Statements come after the statements they reference whenever the references are
/// acyclic, so a bundle reads top-down from evidence to beliefs. Build one with
/// [`crate::read::bundle`] (`View::bundle` in the facade) and write it with
/// [`crate::Tx::import_bundle`].
///
/// ```
/// use tm_core::{BTerm, Bundle, BundleStatement, Valid, Value};
///
/// let iri = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
/// let b = Bundle {
///     root: 0,
///     statements: vec![
///         BundleStatement { local: 0, s: BTerm::Value(iri("alice")), p: iri("worksAt"),
///                           o: BTerm::Value(iri("acme")), valid: Valid::ALWAYS },
///         BundleStatement { local: 1, s: BTerm::Stmt(0), p: iri("confidence"),
///                           o: BTerm::Value(Value::Double(0.8)), valid: Valid::ALWAYS },
///     ],
/// };
/// b.check()?;
/// assert_eq!(b.root_statement().unwrap().p, iri("worksAt"));
/// # Ok::<(), tm_core::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Bundle {
    /// The bundle-local id of the root statement.
    pub root: u32,
    /// The statements, references first.
    pub statements: Vec<BundleStatement>,
}

/// Where one bundle statement landed on import.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ImportedStatement {
    /// The bundle-local id.
    pub local: u32,
    /// The eid in the importing database.
    pub eid: Eid,
    /// False when an existing live statement was reused (idempotent assert).
    pub new: bool,
}

/// The result of [`crate::Tx::import_bundle`]: every bundle statement mapped to its
/// eid, in bundle order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportReport {
    /// The eid of the root statement.
    pub root: Eid,
    /// One entry per bundle statement, in bundle order.
    pub statements: Vec<ImportedStatement>,
}

impl ImportReport {
    /// The eid that bundle-local id `local` was imported as.
    pub fn eid(&self, local: u32) -> Option<Eid> {
        self.statements
            .iter()
            .find(|s| s.local == local)
            .map(|s| s.eid)
    }
}

fn malformed(reason: impl Into<String>) -> Error {
    Error::InvalidTerm {
        position: Position::Value,
        reason: format!("malformed bundle: {}", reason.into()),
    }
}

/// True when `v` is an ObjectId-local value: a skolem IRI or a node, blank node,
/// statement or transaction value, which would alias an id of the importing file.
fn is_local_id(v: &Value) -> bool {
    matches!(
        v.canonical(),
        Value::Node(_) | Value::BNode(_) | Value::Stmt(_) | Value::Tx(_)
    )
}

impl Bundle {
    /// The root statement, if the root id names one.
    pub fn root_statement(&self) -> Option<&BundleStatement> {
        self.statements.iter().find(|s| s.local == self.root)
    }

    /// Checks the structure: unique local ids, a root that exists, statement
    /// references that name statements of the bundle, IRI predicates, and no value
    /// that is a skolem IRI or a local id of a file (statement, node, blank node,
    /// transaction). A reference cycle is allowed here; import refuses it.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidTerm`] naming the first problem.
    pub fn check(&self) -> Result<()> {
        let mut ids = HashSet::new();
        for st in &self.statements {
            if !ids.insert(st.local) {
                return Err(malformed(format!("duplicate id {}", st.local)));
            }
        }
        if !ids.contains(&self.root) {
            return Err(malformed(format!("root {} is not a statement", self.root)));
        }
        for st in &self.statements {
            crate::codec::encode(&st.p).check_origin()?;
            match &st.p {
                Value::Iri(_) if !is_local_id(&st.p) => {}
                other => {
                    return Err(malformed(format!(
                        "predicate {other} of statement {} is not an IRI",
                        st.local
                    )))
                }
            }
            for t in [&st.s, &st.o] {
                match t {
                    BTerm::Stmt(r) if !ids.contains(r) => {
                        return Err(malformed(format!(
                            "statement {} references unknown statement {r}",
                            st.local
                        )))
                    }
                    BTerm::Value(v) if is_local_id(v) => {
                        // a foreign origin is reserved, as on every other input
                        crate::codec::encode(v).check_origin()?;
                        return Err(malformed(format!(
                            "statement {} names {v}, an id local to another database",
                            st.local
                        )));
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Positions in an order where every statement follows the statements it
    /// references (ties by position), or `None` on a reference cycle.
    pub(crate) fn import_order(&self) -> Option<Vec<usize>> {
        let pos: HashMap<u32, usize> = self
            .statements
            .iter()
            .enumerate()
            .map(|(i, s)| (s.local, i))
            .collect();
        let refs = |st: &BundleStatement| -> Vec<usize> {
            [&st.s, &st.o]
                .into_iter()
                .filter_map(|t| match t {
                    BTerm::Stmt(r) => pos.get(r).copied(),
                    _ => None,
                })
                .collect()
        };
        let order = topological(self.statements.len(), |i| refs(&self.statements[i]));
        (order.len() == self.statements.len()).then_some(order)
    }
}

/// Kahn's algorithm over `0..n`, where `refs(i)` lists the nodes `i` must follow.
/// Ready nodes are taken smallest first; nodes on or behind a cycle are left out.
fn topological(n: usize, refs: impl Fn(usize) -> Vec<usize>) -> Vec<usize> {
    let mut pending = vec![0usize; n];
    let mut users: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, p) in pending.iter_mut().enumerate() {
        let mut rs = refs(i);
        rs.sort_unstable();
        rs.dedup();
        *p = rs.len();
        for r in rs {
            users[r].push(i);
        }
    }
    let mut ready: BinaryHeap<Reverse<usize>> =
        (0..n).filter(|i| pending[*i] == 0).map(Reverse).collect();
    let mut out = Vec::with_capacity(n);
    while let Some(Reverse(i)) = ready.pop() {
        out.push(i);
        for &u in &users[i] {
            pending[u] -= 1;
            if pending[u] == 0 {
                ready.push(Reverse(u));
            }
        }
    }
    out
}

/// One statement row read under the view.
#[derive(Copy, Clone)]
struct Row {
    s: ObjectId,
    p: ObjectId,
    o: ObjectId,
    valid: Valid,
}

impl Row {
    /// The statements this row references in subject or object position.
    fn refs(&self) -> impl Iterator<Item = Eid> {
        [self.s, self.o].into_iter().filter_map(Eid::from_oid)
    }
}

fn load_row(exec: &mut dyn Executor, spec: &ViewSpec, eid: Eid) -> Result<Option<Row>> {
    let mut params = Params::new();
    let e = params.push(eid.oid().raw());
    let mut conds = vec![format!("a.eid = {e}")];
    let time = scan_predicates(spec, "a", &mut params);
    if !time.is_empty() {
        conds.push(time);
    }
    let sql = format!(
        "SELECT a.s, a.p, a.o, a.v_from, a.v_to FROM triple a WHERE {}",
        conds.join(" AND ")
    );
    Ok(exec.first_row(&sql, params.values())?.map(|r| {
        let id = |k: usize| ObjectId::from_raw(r[k].as_i64().unwrap_or(0));
        Row {
            s: id(0),
            p: id(1),
            o: id(2),
            valid: Valid {
                from: r[3].as_i64(),
                to: r[4].as_i64(),
            },
        }
    }))
}

/// Why a statement cannot travel in a bundle.
#[derive(Clone, Debug)]
enum Exclusion {
    /// Its subject or object is a transaction (local to the file).
    Transaction,
    /// Its predicate is engine bookkeeping a user write cannot repeat.
    Engine(String),
    /// It references a statement that is not visible in the view.
    Invisible(Eid),
    /// It references an excluded statement.
    Excluded(Eid),
}

impl Exclusion {
    fn feature(&self) -> String {
        match self {
            Exclusion::Transaction => "bundle root that references a transaction".to_string(),
            Exclusion::Engine(p) => format!("bundle root with the engine predicate {p}"),
            Exclusion::Invisible(e) => {
                format!("bundle root that references {e}, which is not in the view")
            }
            Exclusion::Excluded(e) => {
                format!("bundle root that references {e}, which cannot be bundled")
            }
        }
    }
}

/// The bundle of `root` in `spec` (see [`crate::read::bundle`]).
// @lat: [[data-model#Fact Bundles]]
pub(crate) fn export(exec: &mut dyn Executor, spec: &ViewSpec, root: Eid) -> Result<Bundle> {
    if !visible(exec, spec, root)? {
        return Err(Error::NotLive(root));
    }
    let terms = TermReader::new(256);
    // 1. candidates: the dependents, then the downward closure over every candidate
    let mut order = dependents(exec, spec, root)?;
    let deps = order.len();
    let mut candidates: HashSet<Eid> = order.iter().copied().collect();
    let mut rows: HashMap<Eid, Row> = HashMap::new();
    let mut invisible: HashMap<Eid, Eid> = HashMap::new();
    let mut i = 0;
    while i < order.len() {
        let e = order[i];
        i += 1;
        // every candidate was seen visible (by the walk or by the probe below)
        let row = load_row(exec, spec, e)?.ok_or(Error::NotLive(e))?;
        rows.insert(e, row);
        for r in row.refs() {
            if candidates.contains(&r) || invisible.contains_key(&e) {
                continue;
            }
            if visible(exec, spec, r)? {
                candidates.insert(r);
                order.push(r);
            } else {
                invisible.insert(e, r);
            }
        }
    }
    // 2. exclusions, propagated to whatever references an excluded statement
    let mut excluded: BTreeMap<Eid, Exclusion> = BTreeMap::new();
    for &e in &order {
        let row = rows[&e];
        if row.s.tag()? == Tag::Tx || row.o.tag()? == Tag::Tx {
            excluded.insert(e, Exclusion::Transaction);
            continue;
        }
        let p = match terms.decode(exec, row.p, true)? {
            Value::Iri(p) => p,
            other => other.to_string(),
        };
        if p != vocab::SYS_IN_GRAPH && reserved::check_predicate(&p, row.s.tag()?).is_err() {
            excluded.insert(e, Exclusion::Engine(p));
            continue;
        }
        if let Some(r) = invisible.get(&e) {
            excluded.insert(e, Exclusion::Invisible(*r));
        }
    }
    loop {
        let more: Vec<(Eid, Eid)> = order
            .iter()
            .filter(|e| !excluded.contains_key(e))
            .filter_map(|e| {
                rows[e]
                    .refs()
                    .find(|r| excluded.contains_key(r))
                    .map(|r| (*e, r))
            })
            .collect();
        if more.is_empty() {
            break;
        }
        for (e, r) in more {
            excluded.insert(e, Exclusion::Excluded(r));
        }
    }
    if let Some(why) = excluded.get(&root) {
        return Err(Error::unsupported(why.feature()));
    }
    // 3. members: the surviving dependents and their downward closure
    let mut members: Vec<Eid> = order[..deps]
        .iter()
        .copied()
        .filter(|e| !excluded.contains_key(e))
        .collect();
    let mut seen: HashSet<Eid> = members.iter().copied().collect();
    let mut i = 0;
    while i < members.len() {
        let row = rows[&members[i]];
        i += 1;
        for r in row.refs() {
            if seen.insert(r) {
                members.push(r);
            }
        }
    }
    // 4. order: references first, ties by eid, cycle members last in eid order
    members.sort();
    let at: HashMap<Eid, usize> = members.iter().enumerate().map(|(i, e)| (*e, i)).collect();
    let mut sorted = topological(members.len(), |i| {
        rows[&members[i]]
            .refs()
            .filter_map(|r| at.get(&r).copied())
            .collect()
    });
    let placed: HashSet<usize> = sorted.iter().copied().collect();
    sorted.extend((0..members.len()).filter(|i| !placed.contains(i)));
    let local: HashMap<Eid, u32> = sorted
        .iter()
        .enumerate()
        .map(|(k, i)| (members[*i], k as u32))
        .collect();
    // 5. terms: values, local statement ids, anonymous labels in order of appearance
    let mut anon: HashMap<ObjectId, u32> = HashMap::new();
    let mut statements = Vec::with_capacity(sorted.len());
    for (k, i) in sorted.iter().enumerate() {
        let row = rows[&members[*i]];
        let mut term = |id: ObjectId| -> Result<BTerm> {
            Ok(match id.tag()? {
                Tag::Stmt => BTerm::Stmt(local[&Eid::from_oid(id).expect("a statement")]),
                Tag::Node | Tag::BNode => {
                    let next = anon.len() as u32;
                    BTerm::Node(*anon.entry(id).or_insert(next))
                }
                _ => BTerm::Value(terms.decode(exec, id, true)?),
            })
        };
        let s = term(row.s)?;
        let o = term(row.o)?;
        statements.push(BundleStatement {
            local: k as u32,
            s,
            p: terms.decode(exec, row.p, true)?,
            o,
            valid: row.valid,
        });
    }
    Ok(Bundle {
        root: local[&root],
        statements,
    })
}
