//! Text, vector and hybrid recall over the storage traits: ports of
//! `tm_core::text::search`, `tm_core::vector::{search, unembedded}` and
//! `tm_core::recall::recall`, with the same ranking policies.
//!
//! Text matching reproduces FTS5 on the expression `tm_core::text::match_expression`
//! builds: each query word is a phrase of its tokens (a trailing `*` makes the last
//! token a prefix), `All` needs every phrase, `Any` at least one, `Phrase` one
//! phrase of every word. Scores are FTS5's `bm25()` with k1 = 1.2 and b = 0.75.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use tm_core::text::{
    compare as text_compare, TextEvidence, TextHit, TextMode, TextQuery, TEXT_INDEX_VERSION,
};
use tm_core::vector::{
    compare as vector_compare, compare_by_evidence, decode_vector, hamming, sign_bits, similarity,
    EmbeddingModel, Metric, VectorEncoding, VectorHit, VectorQuery, VectorResult, DEFAULT_LIMIT,
};
use tm_core::{
    budget, vocab, Eid, Error, ObjectId, RecallHit, RecallQuery, Result, TxId, Value, ViewSpec,
};

use crate::dict::{self, TermCache};
use crate::evidence::EvidenceReader;
use crate::read::{View, ViewTx};
use crate::store::{Pattern, ReadStore, StmtRow};
use crate::text::tokens;

const K1: f64 = 1.2;
const B: f64 = 0.75;

/// One phrase of a text query: consecutive tokens, the last one possibly a prefix.
#[derive(Clone, Debug, PartialEq)]
struct Phrase {
    tokens: Vec<String>,
    prefix: bool,
}

/// The phrases of `text` in `mode`, as `match_expression` would quote them.
fn phrases(text: &str, mode: TextMode) -> Option<Vec<Phrase>> {
    let mut words: Vec<(String, bool)> = Vec::new();
    for w in text.split_whitespace() {
        let (body, prefix) = match w.strip_suffix('*') {
            Some(b) if mode != TextMode::Phrase => (b, true),
            _ => (w, false),
        };
        let body = body.trim_matches('*');
        if body.chars().any(char::is_alphanumeric) {
            words.push((body.to_string(), prefix));
        }
    }
    if words.is_empty() {
        return None;
    }
    let out = match mode {
        TextMode::Phrase => {
            let joined: Vec<&str> = words.iter().map(|(w, _)| w.as_str()).collect();
            vec![Phrase {
                tokens: tokens(&joined.join(" ")),
                prefix: false,
            }]
        }
        _ => words
            .into_iter()
            .map(|(w, prefix)| Phrase {
                tokens: tokens(&w),
                prefix,
            })
            .collect(),
    };
    Some(out)
}

/// The values holding `phrase` and how often it occurs in each.
fn phrase_hits(store: &dyn ReadStore, phrase: &Phrase) -> Result<BTreeMap<i64, u32>> {
    let n = phrase.tokens.len();
    if n == 0 {
        return Ok(BTreeMap::new());
    }
    // positions of each token slot, per value
    let mut slots: Vec<HashMap<i64, Vec<u32>>> = Vec::with_capacity(n);
    for (i, tok) in phrase.tokens.iter().enumerate() {
        let mut m: HashMap<i64, Vec<u32>> = HashMap::new();
        let toks = if phrase.prefix && i == n - 1 {
            store.text_tokens_with_prefix(tok)?
        } else {
            vec![tok.clone()]
        };
        for t in toks {
            for (v, pos) in store.text_postings(&t)? {
                m.entry(v).or_default().extend(pos);
            }
        }
        slots.push(m);
    }
    let mut out = BTreeMap::new();
    for (v, first) in &slots[0] {
        if slots.iter().any(|s| !s.contains_key(v)) {
            continue;
        }
        let rest: Vec<HashSet<u32>> = slots[1..]
            .iter()
            .map(|s| s[v].iter().copied().collect())
            .collect();
        let starts: BTreeSet<u32> = first.iter().copied().collect();
        let count = starts
            .iter()
            .filter(|p| {
                rest.iter()
                    .enumerate()
                    .all(|(k, set)| set.contains(&(**p + k as u32 + 1)))
            })
            .count() as u32;
        if count > 0 {
            out.insert(*v, count);
        }
    }
    Ok(out)
}

/// The values matching `q` with their `bm25()` score (larger is better, the
/// negation of FTS5's value).
// @tg: implements:: [[openspec:redb-storage#Text index parity]] {since: "2026-10-27"}
pub fn match_values(
    store: &dyn ReadStore,
    text: &str,
    mode: TextMode,
) -> Result<Option<Vec<(i64, f64)>>> {
    let Some(phrases) = phrases(text, mode) else {
        return Ok(None);
    };
    let (rows, total) = store.text_stats()?;
    let n_rows = rows as f64;
    let avgdl = if rows == 0 {
        1.0
    } else {
        total as f64 / n_rows
    };
    let hits: Vec<BTreeMap<i64, u32>> = phrases
        .iter()
        .map(|p| phrase_hits(store, p))
        .collect::<Result<_>>()?;
    let idf: Vec<f64> = hits
        .iter()
        .map(|h| {
            let n_hit = h.len() as f64;
            let idf = ((n_rows - n_hit + 0.5) / (n_hit + 0.5)).ln();
            if idf <= 0.0 {
                1e-6
            } else {
                idf
            }
        })
        .collect();
    let mut matched: BTreeSet<i64> = BTreeSet::new();
    match mode {
        TextMode::Any => {
            for h in &hits {
                matched.extend(h.keys().copied());
            }
        }
        _ => {
            if let Some((first, rest)) = hits.split_first() {
                for v in first.keys() {
                    if rest.iter().all(|h| h.contains_key(v)) {
                        matched.insert(*v);
                    }
                }
            }
        }
    }
    let mut out = Vec::with_capacity(matched.len());
    for v in matched {
        let d = f64::from(store.text_doc(v)?.map_or(0, |d| d.len));
        let mut score = 0.0;
        for (h, idf) in hits.iter().zip(&idf) {
            let f = f64::from(h.get(&v).copied().unwrap_or(0));
            score += idf * ((f * (K1 + 1.0)) / (f + K1 * (1.0 - B + B * d / avgdl)));
        }
        // FTS5's bm25() returns -score; recall stores its negation, the score
        out.push((v, score));
    }
    Ok(Some(out))
}

/// Checks that text recall can run: the index is built and current.
pub fn check_available(store: &dyn ReadStore) -> Result<()> {
    let version = store.meta("text_index")?.unwrap_or(0);
    if version == 0 {
        return Err(Error::TextIndexUnavailable {
            reason: "the text index was never built (open with `text_index` or rebuild it)"
                .to_string(),
        });
    }
    if version != TEXT_INDEX_VERSION {
        return Err(Error::TextIndexUnavailable {
            reason: format!(
                "the text index has version {version}, this build reads {TEXT_INDEX_VERSION}; rebuild it"
            ),
        });
    }
    Ok(())
}

/// The ids of `sys:inGraph` and the graphs, when graph filtering applies.
fn in_graphs(
    store: &dyn ReadStore,
    view: &View,
    graphs: &Option<Vec<ObjectId>>,
    eid: i64,
    ig: Option<ObjectId>,
) -> Result<bool> {
    let Some(gs) = graphs else {
        return Ok(true);
    };
    let Some(ig) = ig else {
        return Ok(false);
    };
    for m in view.scan(store, Pattern::new(Some(eid), Some(ig.raw()), None))? {
        if gs.iter().any(|g| g.raw() == m.as_ref().map_or(0, |m| m.o)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Text recall in `spec` (see `tm_core::text::search`).
pub fn text_search(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    q: &TextQuery,
    terms: &TermCache,
    use_cache: bool,
) -> Result<Vec<TextHit>> {
    check_available(store)?;
    let Some(values) = match_values(store, &q.text, q.mode)? else {
        return Err(Error::invalid_query(format!(
            "text recall needs at least one word, got {:?}",
            q.text
        )));
    };
    if matches!(&q.graphs, Some(g) if g.is_empty())
        || matches!(&q.predicates, Some(p) if p.is_empty())
    {
        return Ok(Vec::new());
    }
    let view = View::resolve(store, spec)?;
    let ig = match &q.graphs {
        Some(_) => match dict::encode(store, &Value::iri(vocab::SYS_IN_GRAPH))? {
            Some(ig) => Some(ig),
            None => return Ok(Vec::new()),
        },
        None => None,
    };
    let mut rows: Vec<(StmtRow, f64)> = Vec::new();
    for (v, score) in values {
        for r in view.scan(store, Pattern::new(None, None, Some(v)))? {
            let r = r?;
            if let Some(ps) = &q.predicates {
                if !ps.iter().any(|p| p.raw() == r.p) {
                    continue;
                }
            }
            if !in_graphs(store, &view, &q.graphs, r.eid, ig)? {
                continue;
            }
            rows.push((r, score));
        }
    }
    rows.sort_by_key(|(r, _)| r.eid);
    let ev = EvidenceReader::new(store, view, q.confidence)?;
    let mut hits = Vec::with_capacity(rows.len());
    for (r, score) in rows {
        budget::check()?;
        let o = ObjectId::from_raw(r.o);
        let (text, lang) = match terms.decode(store, o, use_cache)? {
            Value::Str(s) => (s, None),
            Value::LangStr { lex, lang } => (lex, Some(lang)),
            other => (other.to_string(), None),
        };
        let eid = Eid::from_oid(ObjectId::from_raw(r.eid)).unwrap_or(Eid::new(0));
        let added_at = store.tx_instant(r.t_add)?.unwrap_or(0);
        let evidence = ev.evidence(store, eid, TxId(r.t_add as u64), added_at, terms, use_cache)?;
        hits.push(TextHit {
            eid,
            s: ObjectId::from_raw(r.s),
            p: ObjectId::from_raw(r.p),
            o,
            text,
            lang,
            lexical: score,
            rank: 0,
            evidence,
        });
    }
    hits.sort_by(text_compare);
    for (i, h) in hits.iter_mut().enumerate() {
        h.rank = i as u64 + 1;
    }
    if let Some(l) = q.limit {
        hits.truncate(l);
    }
    if budget::active() {
        budget::charge_rows(hits.len() as u64)?;
        let bytes: u64 = hits.iter().map(|h| 96 + h.text.len() as u64).sum();
        budget::charge_bytes(bytes)?;
    }
    Ok(hits)
}

/// The definition of `model` in `spec` (see `tm_core::vector::model_definition`).
pub fn model_definition(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    model: ObjectId,
    terms: &TermCache,
    use_cache: bool,
) -> Result<Option<EmbeddingModel>> {
    let view = View::resolve(store, spec)?;
    let view = View {
        valid_at: None,
        ..view
    };
    let one = |pred: &str| -> Result<Option<Value>> {
        let Some(pid) = dict::encode(store, &Value::iri(pred))? else {
            return Ok(None);
        };
        let rows = view.scan_eid(
            store,
            Pattern::new(Some(model.raw()), Some(pid.raw()), None),
        )?;
        let Some(r) = rows.last() else {
            return Ok(None);
        };
        terms
            .decode(store, ObjectId::from_raw(r.o), use_cache)
            .map(Some)
    };
    let Some(Value::Int(dims)) = one(vocab::SYS_DIMENSIONS)? else {
        return Ok(None);
    };
    let metric = match one(vocab::SYS_METRIC)? {
        Some(Value::Str(s)) => Metric::from_name(&s),
        _ => None,
    };
    let encoding = match one(vocab::SYS_VECTOR_ENCODING)? {
        Some(Value::Str(s)) => VectorEncoding::from_name(&s),
        _ => None,
    };
    let (Some(metric), Some(encoding)) = (metric, encoding) else {
        return Ok(None);
    };
    Ok(u32::try_from(dims).ok().map(|dimensions| EmbeddingModel {
        dimensions,
        metric,
        encoding,
    }))
}

/// An as-of view sees only the embeddings stored by then.
fn embedding_visible(view: &View, t: i64) -> bool {
    match view.tx {
        ViewTx::AsOf(at) => t <= at,
        _ => true,
    }
}

struct Candidate {
    eid: Eid,
    similarity: f64,
}

fn keep_best(cands: &mut Vec<Candidate>, n: usize) {
    if cands.len() <= n {
        return;
    }
    if n == 0 {
        cands.clear();
        return;
    }
    cands.sort_by(|a, b| {
        b.similarity
            .total_cmp(&a.similarity)
            .then_with(|| a.eid.cmp(&b.eid))
    });
    let edge = cands[n - 1].similarity;
    let keep = cands
        .iter()
        .position(|c| c.similarity < edge)
        .unwrap_or(cands.len());
    cands.truncate(keep);
}

/// Vector recall in `spec` (see `tm_core::vector::search`).
pub fn vector_search(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    q: &VectorQuery,
    terms: &TermCache,
    use_cache: bool,
) -> Result<VectorResult> {
    let Some(model) = q.model else {
        return Err(Error::invalid_query("a vector query needs a model"));
    };
    let def = model_definition(store, spec, model, terms, use_cache)?.ok_or_else(|| {
        Error::invalid_query(format!(
            "{model} is not a defined embedding model in this view (define it with \
             define_embedding_model)"
        ))
    })?;
    let query = def.prepare(&q.vector, "the query vector")?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    if matches!(&q.graphs, Some(g) if g.is_empty())
        || matches!(&q.predicates, Some(p) if p.is_empty())
    {
        return Ok(VectorResult {
            exact: true,
            ..VectorResult::default()
        });
    }
    let view = View::resolve(store, spec)?;
    let ig = match &q.graphs {
        Some(_) => match dict::encode(store, &Value::iri(vocab::SYS_IN_GRAPH))? {
            Some(ig) => Some(ig),
            None => {
                return Ok(VectorResult {
                    exact: true,
                    ..VectorResult::default()
                })
            }
        },
        None => None,
    };
    let dims = def.dimensions as usize;
    let exact = q.prefilter.is_none();
    let qbits = sign_bits(&query);
    let mut cands = Vec::new();
    let mut candidates = 0u64;
    let mut rows: HashMap<Eid, StmtRow> = HashMap::new();
    for e in store.embeddings(model.raw())? {
        let e = e?;
        if !embedding_visible(&view, e.t) {
            continue;
        }
        let Some(r) = store.statement(e.eid)? else {
            continue;
        };
        if !view.selects(&r) {
            continue;
        }
        if let Some(ps) = &q.predicates {
            if !ps.iter().any(|p| p.raw() == r.p) {
                continue;
            }
        }
        if !in_graphs(store, &view, &q.graphs, r.eid, ig)? {
            continue;
        }
        budget::poll()?;
        candidates += 1;
        let score = if exact {
            let Some(v) = decode_vector(&e.vec, def.encoding, dims) else {
                return Err(Error::InvalidTerm {
                    position: tm_core::Position::Object,
                    reason: format!(
                        "the stored vector of statement {} does not match its model",
                        e.eid
                    ),
                });
            };
            similarity(def.metric, &query, &v)
        } else {
            -f64::from(hamming(&e.bits, &qbits))
        };
        let eid = Eid::from_oid(ObjectId::from_raw(e.eid)).unwrap_or(Eid::new(0));
        rows.insert(eid, r);
        cands.push(Candidate {
            eid,
            similarity: score,
        });
    }
    if let Some(n) = q.prefilter {
        keep_best(&mut cands, n.max(limit));
        for c in &mut cands {
            budget::poll()?;
            if let Some(e) = store.embedding(model.raw(), c.eid.oid().raw())? {
                if let Some(v) = decode_vector(&e.vec, def.encoding, dims) {
                    c.similarity = similarity(def.metric, &query, &v);
                }
            }
        }
    }
    keep_best(&mut cands, limit);
    let ev = EvidenceReader::new(store, view, q.confidence)?;
    let mut hits = Vec::with_capacity(cands.len());
    for c in cands {
        budget::check()?;
        let r = rows[&c.eid];
        let added_at = store.tx_instant(r.t_add)?.unwrap_or(0);
        let evidence = ev.evidence(
            store,
            c.eid,
            TxId(r.t_add as u64),
            added_at,
            terms,
            use_cache,
        )?;
        hits.push(VectorHit {
            eid: c.eid,
            s: ObjectId::from_raw(r.s),
            p: ObjectId::from_raw(r.p),
            o: ObjectId::from_raw(r.o),
            similarity: c.similarity,
            rank: 0,
            evidence,
        });
    }
    hits.sort_by(vector_compare);
    hits.truncate(limit);
    for (i, h) in hits.iter_mut().enumerate() {
        h.rank = i as u64 + 1;
    }
    if budget::active() {
        budget::charge_rows(hits.len() as u64)?;
        budget::charge_bytes(hits.len() as u64 * 96)?;
    }
    Ok(VectorResult {
        hits,
        candidates,
        exact,
    })
}

/// The statements of `spec` without an embedding under `model`, in eid order.
pub fn unembedded(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    model: ObjectId,
    limit: Option<usize>,
) -> Result<Vec<Eid>> {
    let view = View::resolve(store, spec)?;
    let mut out = Vec::new();
    for r in view.scan_eid(store, Pattern::ANY)? {
        if limit.is_some_and(|l| out.len() >= l) {
            break;
        }
        budget::poll()?;
        let has = store
            .embedding(model.raw(), r.eid)?
            .is_some_and(|e| embedding_visible(&view, e.t));
        if !has {
            if let Some(e) = Eid::from_oid(ObjectId::from_raw(r.eid)) {
                out.push(e);
            }
        }
    }
    if budget::active() {
        budget::charge_rows(out.len() as u64)?;
        budget::charge_bytes(out.len() as u64 * 8)?;
    }
    Ok(out)
}

/// Hybrid recall in `spec` (see `tm_core::recall::recall`).
pub fn recall(
    store: &dyn ReadStore,
    spec: &ViewSpec,
    q: &RecallQuery,
    terms: &TermCache,
    use_cache: bool,
) -> Result<Vec<RecallHit>> {
    if q.text.is_none() && q.vector.is_none() {
        return Err(Error::invalid_query(
            "a recall needs a text query, a vector query, or both",
        ));
    }
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
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
        for h in text_search(store, spec, &tq, terms, use_cache)? {
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
        for h in vector_search(store, spec, &vq, terms, use_cache)?.hits {
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
        let view = View::resolve(store, spec)?;
        let ev = EvidenceReader::new(store, view, None)?;
        let mut seen: HashSet<Eid> = hits.iter().map(|h| h.eid).collect();
        let mut extra = Vec::new();
        let mut about: HashMap<ObjectId, Vec<StmtRow>> = HashMap::new();
        for h in &hits {
            for subject in [h.eid.oid(), h.s] {
                let cached = match about.entry(subject) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => {
                        let mut rows =
                            view.scan_eid(store, Pattern::new(Some(subject.raw()), None, None))?;
                        rows.truncate(q.expand + 1);
                        e.insert(rows)
                    }
                };
                let rows: Vec<StmtRow> = cached
                    .iter()
                    .filter(|r| r.eid != h.eid.oid().raw())
                    .take(q.expand)
                    .copied()
                    .collect();
                for r in rows {
                    let Some(eid) = Eid::from_oid(ObjectId::from_raw(r.eid)) else {
                        continue;
                    };
                    if !seen.insert(eid) {
                        continue;
                    }
                    budget::check()?;
                    let added_at = store.tx_instant(r.t_add)?.unwrap_or(0);
                    let evidence =
                        ev.evidence(store, eid, TxId(r.t_add as u64), added_at, terms, use_cache)?;
                    extra.push(RecallHit {
                        eid,
                        s: ObjectId::from_raw(r.s),
                        p: ObjectId::from_raw(r.p),
                        o: ObjectId::from_raw(r.o),
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
