//! Hybrid recall: text and vector recall fused by reciprocal rank, with optional
//! expansion to each hit's layers and to facts about its subject (OpenSpec change
//! `add-hybrid-recall`).
//!
//! Each channel runs its own exact, view-aware recall ([`crate::text::search`],
//! [`crate::vector::search`]) for `depth` candidates in the same snapshot. A hit's
//! fused score is the sum over the channels that found it of `1 / (rrf_k + rank)`
//! (Cormack, Clarke and Büttcher, SIGIR 2009); scores from different channels are
//! never compared directly. Ties are broken by the evidence the store holds, never
//! by an invented score.

use std::collections::HashMap;

use crate::budget;
use crate::error::{Error, Result};
use crate::exec::{Executor, Params};
use crate::id::{Eid, ObjectId, TxId};
use crate::term::TermReader;
use crate::text::{self, EvidenceReader, TextEvidence, TextQuery};
use crate::vector::{self, compare_by_evidence, VectorQuery};
use crate::view::{scan_predicates, ViewSpec};

/// The name of the ranking policy [`recall`] applies.
pub const RANK_POLICY: &str = "tiramemsu-recall-rank/1";

/// The default `rrf_k` of reciprocal rank fusion.
pub const DEFAULT_RRF_K: u32 = 60;

/// One hybrid recall. At least one of `text` and `vector` is required; their own
/// `limit`s are ignored in favour of `depth`.
#[derive(Clone, Debug, PartialEq)]
pub struct RecallQuery {
    /// The text channel.
    pub text: Option<TextQuery>,
    /// The vector channel.
    pub vector: Option<VectorQuery>,
    /// At most this many fused hits (expansions come on top). `None` = 10.
    pub limit: Option<usize>,
    /// Candidates taken from each channel. `None` = `max(4 × limit, 50)`.
    pub depth: Option<usize>,
    /// Per fused hit, add up to this many of its visible layer statements (subject
    /// = the hit's eid) and up to this many visible facts about its subject node.
    /// 0 = no expansion.
    pub expand: usize,
    /// The constant of reciprocal rank fusion.
    pub rrf_k: u32,
}

impl Default for RecallQuery {
    fn default() -> Self {
        RecallQuery {
            text: None,
            vector: None,
            limit: None,
            depth: None,
            expand: 0,
            rrf_k: DEFAULT_RRF_K,
        }
    }
}

/// One recalled statement. Fused hits come first, ordered by [`RANK_POLICY`]:
/// fused score descending, then confidence (absent last), confirmations, authors,
/// `added_at` newer first, eid ascending. Expanded statements follow, in the order
/// of the hit they were reached from (`via`), layers before subject facts, each
/// group in eid order.
#[derive(Clone, Debug, PartialEq)]
pub struct RecallHit {
    /// The statement.
    pub eid: Eid,
    /// Its subject.
    pub s: ObjectId,
    /// Its predicate.
    pub p: ObjectId,
    /// Its object.
    pub o: ObjectId,
    /// The fused score; 0 for an expanded statement.
    pub score: f64,
    /// 1-based position in the result.
    pub rank: u64,
    /// The text channel's lexical score, when it found the statement.
    pub lexical: Option<f64>,
    /// The vector channel's similarity, when it found the statement.
    pub similarity: Option<f64>,
    /// The statement's rank in the text channel.
    pub text_rank: Option<u64>,
    /// The statement's rank in the vector channel.
    pub vector_rank: Option<u64>,
    /// The evidence, read in the query's view.
    pub evidence: TextEvidence,
    /// For an expanded statement: the fused hit it was reached from.
    pub via: Option<Eid>,
}

/// Runs a hybrid recall in the view `spec`. Every query runs on `exec`, in one
/// snapshot.
///
/// # Errors
///
/// `InvalidQuery` when neither channel is given, and every error of the channels
/// (for example `TextIndexUnavailable` when the text index is not built).
// @lat: [[query#Hybrid Recall]]
pub fn recall(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    q: &RecallQuery,
    terms: &TermReader,
    use_cache: bool,
) -> Result<Vec<RecallHit>> {
    if q.text.is_none() && q.vector.is_none() {
        return Err(Error::invalid_query(
            "a recall needs a text query, a vector query, or both",
        ));
    }
    let limit = q.limit.unwrap_or(vector::DEFAULT_LIMIT);
    let depth = q.depth.unwrap_or_else(|| (limit * 4).max(50));
    let k = f64::from(q.rrf_k);
    let mut fused: HashMap<Eid, RecallHit> = HashMap::new();
    let entry = |eid, s, p, o, evidence: &TextEvidence| RecallHit {
        eid,
        s,
        p,
        o,
        score: 0.0,
        rank: 0,
        lexical: None,
        similarity: None,
        text_rank: None,
        vector_rank: None,
        evidence: evidence.clone(),
        via: None,
    };
    if let Some(tq) = &q.text {
        let tq = TextQuery {
            limit: Some(depth),
            ..tq.clone()
        };
        for h in text::search(exec, spec, &tq, terms, use_cache)? {
            let e = fused
                .entry(h.eid)
                .or_insert_with(|| entry(h.eid, h.s, h.p, h.o, &h.evidence));
            e.lexical = Some(h.lexical);
            e.text_rank = Some(h.rank);
            e.score += 1.0 / (k + h.rank as f64);
        }
    }
    if let Some(vq) = &q.vector {
        let vq = VectorQuery {
            limit: Some(depth),
            ..vq.clone()
        };
        for h in vector::search(exec, spec, &vq, terms, use_cache)?.hits {
            let e = fused
                .entry(h.eid)
                .or_insert_with(|| entry(h.eid, h.s, h.p, h.o, &h.evidence));
            e.similarity = Some(h.similarity);
            e.vector_rank = Some(h.rank);
            e.score += 1.0 / (k + h.rank as f64);
        }
    }
    let mut hits: Vec<RecallHit> = fused.into_values().collect();
    hits.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| compare_by_evidence((&a.evidence, a.eid), (&b.evidence, b.eid)))
    });
    hits.truncate(limit);
    if q.expand > 0 {
        let ev = EvidenceReader::new(exec, spec, None)?;
        let mut seen: std::collections::HashSet<Eid> = hits.iter().map(|h| h.eid).collect();
        let mut extra = Vec::new();
        // one query per distinct subject: n + 1 rows cover the first n statements
        // other than the hit itself
        let mut about: HashMap<ObjectId, Vec<Row>> = HashMap::new();
        for h in &hits {
            for subject in [h.eid.oid(), h.s] {
                let cached = match about.entry(subject) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(statements_about(exec, spec, subject, q.expand + 1)?)
                    }
                };
                let rows: Vec<Row> = cached
                    .iter()
                    .filter(|r| r.0 != h.eid)
                    .take(q.expand)
                    .copied()
                    .collect();
                for (eid, s, p, o, t_add, added_at) in rows {
                    if !seen.insert(eid) {
                        continue;
                    }
                    budget::check()?;
                    let evidence = ev.evidence(exec, eid, t_add, added_at, terms, use_cache)?;
                    extra.push(RecallHit {
                        eid,
                        s,
                        p,
                        o,
                        score: 0.0,
                        rank: 0,
                        lexical: None,
                        similarity: None,
                        text_rank: None,
                        vector_rank: None,
                        evidence,
                        via: Some(h.eid),
                    });
                }
            }
        }
        hits.extend(extra);
    }
    for (i, h) in hits.iter_mut().enumerate() {
        h.rank = i as u64 + 1;
    }
    if budget::active() {
        budget::charge_rows(hits.len() as u64)?;
        budget::charge_bytes(hits.len() as u64 * 112)?;
    }
    Ok(hits)
}

type Row = (Eid, ObjectId, ObjectId, ObjectId, TxId, i64);

/// Up to `n` statements visible in `spec` whose subject is `subject`, in eid order.
fn statements_about(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    subject: ObjectId,
    n: usize,
) -> Result<Vec<Row>> {
    let mut params = Params::new();
    let s = params.push(subject.raw());
    let mut conds = vec![format!("t.s = +{s}")];
    let time = scan_predicates(spec, "t", &mut params);
    if !time.is_empty() {
        conds.push(time);
    }
    let lim = params.push(i64::try_from(n).unwrap_or(i64::MAX));
    let sql = format!(
        "SELECT t.eid, t.s, t.p, t.o, t.t_add, (SELECT instant FROM tx WHERE tx.t = t.t_add) \
         FROM triple AS t WHERE {} ORDER BY t.eid LIMIT {lim}",
        conds.join(" AND ")
    );
    let mut out = Vec::new();
    exec.query(&sql, params.values(), &mut |r| {
        let i = |k: usize| r[k].as_i64().unwrap_or(0);
        if let Some(eid) = Eid::from_oid(ObjectId::from_raw(i(0))) {
            out.push((
                eid,
                ObjectId::from_raw(i(1)),
                ObjectId::from_raw(i(2)),
                ObjectId::from_raw(i(3)),
                TxId(i(4) as u64),
                i(5),
            ));
        }
        Ok(())
    })?;
    Ok(out)
}
