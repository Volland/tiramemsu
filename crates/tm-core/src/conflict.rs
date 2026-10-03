//! Conflict inspection: simultaneous disagreement in one view (OpenSpec change
//! `add-memory-conflict-review`).
//!
//! A conflict is a subject/predicate pair with at least two distinct objects whose
//! half-open valid intervals overlap, among the statements a view selects. It is
//! reported as *potential* disagreement with the attributed evidence of every
//! statement involved; nothing is scored, resolved or written. Parallel statements
//! with the same object support one value and never conflict with each other.
//!
//! The read runs on the caller's connection in one snapshot: one self-join finds
//! the candidate pairs (the same overlap test as assert), then each pair's
//! statements are read and swept over their interval bounds.

use std::collections::{BTreeSet, HashSet};

use crate::budget;
use crate::error::{Error, Result};
use crate::exec::{Executor, Params};
use crate::id::{Eid, ObjectId, Tag, TxId};
use crate::report::Valid;
use crate::term::TermReader;
use crate::text::EvidenceReader;
use crate::value::Value;
use crate::view::{scan_predicates, TxSel, ViewSpec};
use crate::vocab;

/// The predicate read as a statement's source layer when
/// [`ConflictQuery::source`] is `None`: `v:source` in the default vocabulary.
pub const DEFAULT_SOURCE: &str = "urn:tiramemsu:v:source";

/// One conflict inspection: optional filters and the layer predicates to read.
/// `ConflictQuery::default()` inspects every subject and predicate.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictQuery {
    /// Only this subject. `None` = every subject.
    pub subject: Option<ObjectId>,
    /// Only this predicate. `None` = every predicate outside the `sys:` namespace.
    pub predicate: Option<ObjectId>,
    /// At most this many conflicts, in subject/predicate order. `None` = all.
    pub limit: Option<usize>,
    /// The predicate whose numeric objects on a statement are its confidence
    /// layer. `None` = [`crate::text::DEFAULT_CONFIDENCE`].
    pub confidence: Option<ObjectId>,
    /// The predicate whose objects on a statement are its source layer. `None` =
    /// [`DEFAULT_SOURCE`].
    pub source: Option<ObjectId>,
}

/// The attributed evidence of one statement taking part in a conflict, read in
/// the inspection's view. A layer the statement does not have is absent (`None`)
/// or empty, never estimated; no combined score is computed.
#[derive(Clone, Debug, PartialEq)]
pub struct ConflictEvidence {
    /// The statement.
    pub eid: Eid,
    /// Its valid interval, `[from, to)`.
    pub valid: Valid,
    /// The transaction that asserted it.
    pub t_add: TxId,
    /// That transaction's instant (epoch ms).
    pub added_at: i64,
    /// The largest numeric object of its visible confidence statements; `None`
    /// when it has none.
    pub confidence: Option<f64>,
    /// The transactions of its visible `sys:confirmedBy` statements, in eid order.
    pub confirmed_by: Vec<TxId>,
    /// Distinct `sys:author` values of the asserting and confirming transactions.
    pub authors: Vec<ObjectId>,
    /// Distinct `sys:source` values of the asserting and confirming transactions.
    pub sources: Vec<ObjectId>,
    /// The objects of its visible source-layer statements
    /// ([`ConflictQuery::source`]), in eid order.
    pub source_layer: Vec<ObjectId>,
}

/// One of the disagreeing objects, with every statement that asserts it inside a
/// conflict window. Two or more statements here are support for one value.
#[derive(Clone, Debug, PartialEq)]
pub struct ConflictValue {
    /// The object.
    pub o: ObjectId,
    /// Its statements that overlap a conflict window, ascending eid.
    pub statements: Vec<ConflictEvidence>,
}

/// A subject/predicate pair with distinct objects valid at the same time.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    /// The subject.
    pub s: ObjectId,
    /// The predicate.
    pub p: ObjectId,
    /// True when the predicate is declared `sys:cardinality sys:many` in the view:
    /// several values may all be intended. Either way a conflict is potential
    /// disagreement, never a schema violation.
    pub declared_many: bool,
    /// The maximal valid intervals in which two or more distinct objects hold,
    /// ascending and disjoint.
    pub overlaps: Vec<Valid>,
    /// The disagreeing objects, in order of their first statement's eid.
    pub values: Vec<ConflictValue>,
}

/// One candidate statement of a subject/predicate pair.
struct Row {
    eid: Eid,
    o: ObjectId,
    valid: Valid,
    t_add: TxId,
    added_at: i64,
}

/// True when `v` covers the whole segment `[a, b)` (`None` = unbounded).
fn covers(v: &Valid, a: Option<i64>, b: Option<i64>) -> bool {
    let start = match (v.from, a) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(f), Some(a)) => f <= a,
    };
    let end = match (v.to, b) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(t), Some(b)) => t >= b,
    };
    start && end
}

/// True when the half-open intervals `v` and `w` intersect.
fn intersects(v: &Valid, w: &Valid) -> bool {
    let lt = |a: Option<i64>, b: Option<i64>| match (a, b) {
        (Some(a), Some(b)) => a < b,
        _ => true,
    };
    lt(v.from, w.to) && lt(w.from, v.to)
}

/// The maximal intervals in which two or more distinct objects of `rows` hold:
/// a sweep over the elementary segments between every finite bound.
fn windows(rows: &[Row]) -> Vec<Valid> {
    let mut points: Vec<i64> = rows
        .iter()
        .flat_map(|r| [r.valid.from, r.valid.to])
        .flatten()
        .collect();
    points.sort_unstable();
    points.dedup();
    let mut bounds: Vec<Option<i64>> = Vec::with_capacity(points.len() + 2);
    bounds.push(None);
    bounds.extend(points.into_iter().map(Some));
    bounds.push(None);
    let mut out: Vec<Valid> = Vec::new();
    for seg in bounds.windows(2) {
        let (a, b) = (seg[0], seg[1]);
        let objects: HashSet<ObjectId> = rows
            .iter()
            .filter(|r| covers(&r.valid, a, b))
            .map(|r| r.o)
            .collect();
        if objects.len() < 2 {
            continue;
        }
        match out.last_mut() {
            // consecutive segments share their bound
            Some(last) if last.to == a && a.is_some() => last.to = b,
            _ => out.push(Valid { from: a, to: b }),
        }
    }
    out
}

/// The conflicts of `spec` that match `q` (see [`Conflict`]). Read-only: it never
/// retracts, supersedes or confirms anything.
///
/// Every statement the view selects is considered, so in an as-of view a
/// conflict is a disagreement memory held then. Predicates in the `sys:`
/// namespace (schema flags, memberships, confirmations) are skipped unless
/// `q.predicate` names one.
///
/// # Errors
///
/// [`Error::Unsupported`] on the history view, whose statements need not have
/// been believed at the same time (inspect an as-of view instead), and
/// [`Error::Sqlite`] on a read failure.
// @lat: [[query#Conflict Inspection]]
pub fn inspect(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    q: &ConflictQuery,
    terms: &TermReader,
    use_cache: bool,
) -> Result<Vec<Conflict>> {
    if spec.tx == TxSel::History {
        return Err(Error::unsupported(
            "conflict inspection on the history view (use an as-of view)",
        ));
    }
    // 1. candidate pairs: two statements, distinct objects, overlapping valid time
    let mut params = Params::new();
    let mut conds = vec![
        "b.s = a.s".to_string(),
        "b.p = a.p".to_string(),
        "b.o <> a.o".to_string(),
        "b.eid > a.eid".to_string(),
        "(a.v_from IS NULL OR b.v_to IS NULL OR a.v_from < b.v_to)".to_string(),
        "(b.v_from IS NULL OR a.v_to IS NULL OR b.v_from < a.v_to)".to_string(),
    ];
    if let Some(s) = q.subject {
        conds.push(format!("a.s = {}", params.push(s.raw())));
    }
    if let Some(p) = q.predicate {
        conds.push(format!("a.p = {}", params.push(p.raw())));
    }
    for alias in ["a", "b"] {
        let time = scan_predicates(spec, alias, &mut params);
        if !time.is_empty() {
            conds.push(time);
        }
    }
    let sql = format!(
        "SELECT DISTINCT a.s, a.p FROM triple AS a JOIN triple AS b ON {} ORDER BY a.s, a.p",
        conds.join(" AND ")
    );
    let pairs: Vec<(ObjectId, ObjectId)> = exec
        .rows(&sql, params.values())?
        .into_iter()
        .map(|r| {
            let id = |k: usize| ObjectId::from_raw(r[k].as_i64().unwrap_or(0));
            (id(0), id(1))
        })
        .collect();
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let ev = EvidenceReader::new(exec, spec, q.confidence)?;
    let source_layer = match q.source {
        Some(p) => Some(p),
        None => TermReader::encode(exec, &Value::iri(DEFAULT_SOURCE))?,
    };
    let sys_source = TermReader::encode(exec, &Value::iri(vocab::SYS_SOURCE))?;
    let cardinality = TermReader::encode(exec, &Value::iri(vocab::SYS_CARDINALITY))?;
    let many = TermReader::encode(exec, &Value::iri(vocab::SYS_MANY))?;
    let mut out = Vec::new();
    for (s, p) in pairs {
        if q.limit.is_some_and(|l| out.len() >= l) {
            break;
        }
        budget::check()?;
        if q.predicate.is_none() {
            if let Value::Iri(iri) = terms.decode(exec, p, use_cache)? {
                if iri.starts_with(vocab::SYS) {
                    continue;
                }
            }
        }
        // 2. the pair's statements, swept over their bounds
        let rows = pair_rows(exec, spec, s, p)?;
        let overlaps = windows(&rows);
        if overlaps.is_empty() {
            continue;
        }
        let mut values: Vec<ConflictValue> = Vec::new();
        for r in rows
            .iter()
            .filter(|r| overlaps.iter().any(|w| intersects(&r.valid, w)))
        {
            let me = r.eid.oid();
            let confirmations = match ev.confirmed_by {
                Some(cb) => ev.objects(exec, me, cb)?,
                None => Vec::new(),
            };
            let confirmed_by: Vec<TxId> = confirmations
                .iter()
                .filter(|o| o.tag_bits() == Tag::Tx as u8)
                .map(|o| TxId(o.unsigned_payload()))
                .collect();
            let txs: Vec<ObjectId> = std::iter::once(r.t_add.oid())
                .chain(confirmed_by.iter().map(|t| t.oid()))
                .collect();
            let mut tx_meta = |p: Option<ObjectId>| -> Result<Vec<ObjectId>> {
                let mut seen = BTreeSet::new();
                let mut list = Vec::new();
                if let Some(p) = p {
                    for t in &txs {
                        for o in ev.objects(exec, *t, p)? {
                            if seen.insert(o) {
                                list.push(o);
                            }
                        }
                    }
                }
                Ok(list)
            };
            let authors = tx_meta(ev.author)?;
            let sources = tx_meta(sys_source)?;
            let evidence = ConflictEvidence {
                eid: r.eid,
                valid: r.valid,
                t_add: r.t_add,
                added_at: r.added_at,
                confidence: ev.confidence(exec, me, terms, use_cache)?,
                confirmed_by,
                authors,
                sources,
                source_layer: match source_layer {
                    Some(sl) => ev.objects(exec, me, sl)?,
                    None => Vec::new(),
                },
            };
            match values.iter_mut().find(|v| v.o == r.o) {
                Some(v) => v.statements.push(evidence),
                None => values.push(ConflictValue {
                    o: r.o,
                    statements: vec![evidence],
                }),
            }
        }
        let declared_many = match (cardinality, many) {
            (Some(c), Some(m)) => ev.objects(exec, p, c)?.contains(&m),
            _ => false,
        };
        if budget::active() {
            let n: usize = values.iter().map(|v| v.statements.len()).sum();
            budget::charge_rows(1)?;
            budget::charge_bytes(48 + n as u64 * 96)?;
        }
        out.push(Conflict {
            s,
            p,
            declared_many,
            overlaps,
            values,
        });
    }
    Ok(out)
}

/// The statements of `(s, p, ?)` selected by `spec`, ascending eid.
fn pair_rows(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    s: ObjectId,
    p: ObjectId,
) -> Result<Vec<Row>> {
    let mut params = Params::new();
    let sp = params.push(s.raw());
    let pp = params.push(p.raw());
    let mut sql = format!(
        "SELECT a.eid, a.o, a.v_from, a.v_to, a.t_add, \
         (SELECT instant FROM tx WHERE tx.t = a.t_add) \
         FROM triple AS a WHERE a.s = {sp} AND a.p = {pp}"
    );
    let time = scan_predicates(spec, "a", &mut params);
    if !time.is_empty() {
        sql.push_str(" AND ");
        sql.push_str(&time);
    }
    sql.push_str(" ORDER BY a.eid");
    Ok(exec
        .rows(&sql, params.values())?
        .into_iter()
        .map(|r| {
            let i = |k: usize| r[k].as_i64().unwrap_or(0);
            Row {
                eid: Eid::from_oid(ObjectId::from_raw(i(0))).unwrap_or(Eid::new(0)),
                o: ObjectId::from_raw(i(1)),
                valid: Valid {
                    from: r[2].as_i64(),
                    to: r[3].as_i64(),
                },
                t_add: TxId(i(4) as u64),
                added_at: i(5),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(o: i64, from: Option<i64>, to: Option<i64>) -> Row {
        Row {
            eid: Eid::new(1),
            o: ObjectId::from_raw(o),
            valid: Valid { from, to },
            t_add: TxId(1),
            added_at: 0,
        }
    }

    #[test]
    fn windows_cover_only_simultaneous_distinct_objects() {
        // a [0,10), b [5,15), a [20,30): one window [5,10)
        let rows = [
            row(1, Some(0), Some(10)),
            row(2, Some(5), Some(15)),
            row(1, Some(20), Some(30)),
        ];
        assert_eq!(windows(&rows), vec![Valid::between(5, 10)]);
        // disjoint episodes never conflict
        let rows = [row(1, Some(0), Some(10)), row(2, Some(10), Some(20))];
        assert!(windows(&rows).is_empty());
        // parallel equal objects support each other
        let rows = [row(1, None, None), row(1, Some(3), None)];
        assert!(windows(&rows).is_empty());
        // unbounded overlap, merged across inner bounds
        let rows = [
            row(1, None, None),
            row(2, Some(3), None),
            row(3, Some(5), Some(8)),
        ];
        assert_eq!(windows(&rows), vec![Valid::from(3)]);
        let rows = [row(1, None, None), row(2, None, Some(4))];
        assert_eq!(
            windows(&rows),
            vec![Valid {
                from: None,
                to: Some(4)
            }]
        );
    }
}
