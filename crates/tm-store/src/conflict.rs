//! Conflict inspection over the storage traits. A port of `tm_core::conflict`:
//! same candidates, windows, evidence and order.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use tm_core::conflict::{Conflict, ConflictEvidence, ConflictQuery, ConflictValue, DEFAULT_SOURCE};
use tm_core::vocab;
use tm_core::{budget, Eid, Error, ObjectId, Result, Tag, TxId, TxSel, Valid, Value, ViewSpec};

use crate::dict::{self, TermCache};
use crate::evidence::EvidenceReader;
use crate::read::View;
use crate::store::{Pattern, ReadStore, StmtRow};

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

/// The conflicts of `spec` that match `q` (see `tm_core::conflict::inspect`).
/// Read-only.
///
/// # Errors
///
/// [`Error::Unsupported`] on the history view.
pub fn inspect(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    q: &ConflictQuery,
    terms: &TermCache,
    use_cache: bool,
) -> Result<Vec<Conflict>> {
    if spec.tx == TxSel::History {
        return Err(Error::unsupported(
            "conflict inspection on the history view (use an as-of view)",
        ));
    }
    let view = View::resolve(store, spec)?;
    // 1. candidate pairs: two statements, distinct objects, overlapping valid time
    let mut groups: BTreeMap<(i64, i64), Vec<StmtRow>> = BTreeMap::new();
    for r in view.scan(
        store,
        Pattern::new(
            q.subject.map(ObjectId::raw),
            q.predicate.map(ObjectId::raw),
            None,
        ),
    )? {
        let r = r?;
        groups.entry((r.s, r.p)).or_default().push(r);
    }
    let overlap = |a: &StmtRow, b: &StmtRow| {
        (a.v_from.is_none() || b.v_to.is_none() || a.v_from < b.v_to)
            && (b.v_from.is_none() || a.v_to.is_none() || b.v_from < a.v_to)
    };
    let pairs: Vec<(ObjectId, ObjectId)> = groups
        .into_iter()
        .filter(|(_, rows)| {
            rows.iter().any(|a| {
                rows.iter()
                    .any(|b| b.eid > a.eid && b.o != a.o && overlap(a, b))
            })
        })
        .map(|((s, p), _)| (ObjectId::from_raw(s), ObjectId::from_raw(p)))
        .collect();
    if pairs.is_empty() {
        return Ok(Vec::new());
    }
    let ev = EvidenceReader::new(store, view, q.confidence)?;
    let source_layer = match q.source {
        Some(p) => Some(p),
        None => dict::encode(store, &Value::iri(DEFAULT_SOURCE))?,
    };
    let sys_source = dict::encode(store, &Value::iri(vocab::SYS_SOURCE))?;
    let cardinality = dict::encode(store, &Value::iri(vocab::SYS_CARDINALITY))?;
    let many = dict::encode(store, &Value::iri(vocab::SYS_MANY))?;
    let mut out = Vec::new();
    for (s, p) in pairs {
        if q.limit.is_some_and(|l| out.len() >= l) {
            break;
        }
        budget::check()?;
        if q.predicate.is_none() {
            if let Value::Iri(iri) = terms.decode(store, p, use_cache)? {
                if iri.starts_with(vocab::SYS) {
                    continue;
                }
            }
        }
        // 2. the pair's statements, swept over their bounds
        let rows = pair_rows(store, &view, s, p)?;
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
                Some(cb) => ev.objects(store, me, cb)?,
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
            let tx_meta = |p: Option<ObjectId>| -> Result<Vec<ObjectId>> {
                let mut seen = BTreeSet::new();
                let mut list = Vec::new();
                if let Some(p) = p {
                    for t in &txs {
                        for o in ev.objects(store, *t, p)? {
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
                confidence: ev.confidence(store, me, terms, use_cache)?,
                confirmed_by,
                authors,
                sources,
                source_layer: match source_layer {
                    Some(sl) => ev.objects(store, me, sl)?,
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
            (Some(c), Some(m)) => ev.objects(store, p, c)?.contains(&m),
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

/// The statements of `(s, p, ?)` selected by `view`, ascending eid.
fn pair_rows(store: &dyn ReadStore, view: &View, s: ObjectId, p: ObjectId) -> Result<Vec<Row>> {
    let rows = view.scan_eid(store, Pattern::new(Some(s.raw()), Some(p.raw()), None))?;
    rows.into_iter()
        .map(|r| {
            Ok(Row {
                eid: Eid::from_oid(ObjectId::from_raw(r.eid)).unwrap_or(Eid::new(0)),
                o: ObjectId::from_raw(r.o),
                valid: Valid {
                    from: r.v_from,
                    to: r.v_to,
                },
                t_add: TxId(r.t_add as u64),
                added_at: store.tx_instant(r.t_add)?.unwrap_or(0),
            })
        })
        .collect()
}
