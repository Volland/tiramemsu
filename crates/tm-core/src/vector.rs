//! Vector recall over caller-supplied embeddings (OpenSpec change
//! `add-vector-recall`).
//!
//! Embeddings are source data, not a derived index: one row per (model, statement)
//! in the append-only `embedding` table of format 5, written inside the write
//! transaction by [`Tx::set_embedding`](crate::Tx::set_embedding). The store never
//! calls a model. A model is defined by `sys:` statements on its IRI (dimensions,
//! metric, encoding), written by
//! [`Tx::define_embedding_model`](crate::Tx::define_embedding_model).
//!
//! Search is exact and host-neutral. One SQL statement selects the embeddings of
//! the statements the view selects, through [`scan_predicates`] (the one writer of
//! time predicates) and the same predicate and graph filters as text recall;
//! similarity, the top hits and the ranking are computed here. Approximate indexes
//! filter after their top-k, which loses hits under `as_of`, `valid_at` and graph
//! scopes; scoring only visible rows does not. An as-of view also sees only the
//! embeddings stored by its transaction, so a past view answers the same forever.
//! The optional binary prefilter keeps
//! the best candidates by Hamming distance on the stored sign bits and re-ranks them
//! exactly; its result says it is not exhaustive.

use std::cmp::Ordering;

use crate::budget;
use crate::error::{Error, Result};
use crate::exec::{Executor, Params, SqlValue};
use crate::id::{Eid, ObjectId, TxId};
use crate::term::TermReader;
use crate::text::{EvidenceReader, TextEvidence};
use crate::value::Value;
use crate::view::{scan_predicates, TimeRef, TxSel, ViewSpec};
use crate::vocab;

/// The name of the ranking policy [`search`] applies; see [`VectorHit`].
pub const RANK_POLICY: &str = "tiramemsu-vector-rank/1";

/// The most dimensions a model may declare.
pub const MAX_DIMENSIONS: u32 = 16_384;

/// How two vectors are compared. Similarity is always "larger is better".
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Metric {
    /// Cosine similarity in `[-1, 1]`. Vectors are stored unit-length, so it is the
    /// dot product of the stored and the normalised query vector.
    #[default]
    Cosine,
    /// The raw dot product.
    Dot,
    /// Euclidean distance, reported negated (`-‖a − b‖`).
    L2,
}

impl Metric {
    /// The name stored as `sys:metric` and used by the bindings.
    pub fn name(self) -> &'static str {
        match self {
            Metric::Cosine => "cosine",
            Metric::Dot => "dot",
            Metric::L2 => "l2",
        }
    }

    /// Inverse of [`Metric::name`] (case-insensitive).
    pub fn from_name(s: &str) -> Option<Metric> {
        match s.to_ascii_lowercase().as_str() {
            "cosine" => Some(Metric::Cosine),
            "dot" => Some(Metric::Dot),
            "l2" => Some(Metric::L2),
            _ => None,
        }
    }
}

/// How vector components are stored.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum VectorEncoding {
    /// 4 bytes per component, little-endian IEEE 754 single precision.
    #[default]
    F32,
    /// 2 bytes per component, little-endian IEEE 754 half precision (round to
    /// nearest even). Half the size, with a small loss of precision.
    F16,
}

impl VectorEncoding {
    /// The name stored as `sys:vectorEncoding` and used by the bindings.
    pub fn name(self) -> &'static str {
        match self {
            VectorEncoding::F32 => "f32",
            VectorEncoding::F16 => "f16",
        }
    }

    /// Inverse of [`VectorEncoding::name`] (case-insensitive).
    pub fn from_name(s: &str) -> Option<VectorEncoding> {
        match s.to_ascii_lowercase().as_str() {
            "f32" => Some(VectorEncoding::F32),
            "f16" => Some(VectorEncoding::F16),
            _ => None,
        }
    }

    fn width(self) -> usize {
        match self {
            VectorEncoding::F32 => 4,
            VectorEncoding::F16 => 2,
        }
    }
}

/// The definition of an embedding model, stored as `sys:` statements on its IRI.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EmbeddingModel {
    /// The number of components of every vector (1 to [`MAX_DIMENSIONS`]).
    pub dimensions: u32,
    /// How vectors are compared.
    pub metric: Metric,
    /// How vectors are stored.
    pub encoding: VectorEncoding,
}

impl EmbeddingModel {
    /// A cosine, `f32` model of `dimensions` components.
    pub fn new(dimensions: u32) -> EmbeddingModel {
        EmbeddingModel {
            dimensions,
            metric: Metric::Cosine,
            encoding: VectorEncoding::F32,
        }
    }

    pub(crate) fn check(&self) -> Result<()> {
        if self.dimensions == 0 || self.dimensions > MAX_DIMENSIONS {
            return Err(Error::invalid_query(format!(
                "an embedding model needs 1 to {MAX_DIMENSIONS} dimensions, got {}",
                self.dimensions
            )));
        }
        Ok(())
    }

    /// Checks `v` against the model and returns the vector to store or compare:
    /// unit length for cosine.
    pub(crate) fn prepare(&self, v: &[f32], what: &str) -> Result<Vec<f32>> {
        if v.len() != self.dimensions as usize {
            return Err(Error::invalid_query(format!(
                "{what} has {} components, the model has {} dimensions",
                v.len(),
                self.dimensions
            )));
        }
        if v.iter().any(|x| !x.is_finite()) {
            return Err(Error::invalid_query(format!(
                "{what} has a component that is NaN or infinite"
            )));
        }
        if self.metric != Metric::Cosine {
            return Ok(v.to_vec());
        }
        let norm = v.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
        if norm == 0.0 {
            return Err(Error::invalid_query(format!(
                "{what} is the zero vector, which has no cosine direction"
            )));
        }
        Ok(v.iter().map(|x| (f64::from(*x) / norm) as f32).collect())
    }
}

// ---------------------------------------------------------------------------
// Encoding

/// Packs `v` in `enc` (little-endian).
pub fn encode_vector(v: &[f32], enc: VectorEncoding) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * enc.width());
    for x in v {
        match enc {
            VectorEncoding::F32 => out.extend_from_slice(&x.to_le_bytes()),
            VectorEncoding::F16 => out.extend_from_slice(&f32_to_f16(*x).to_le_bytes()),
        }
    }
    out
}

/// Unpacks a vector stored in `enc`; `None` when the length does not match.
pub fn decode_vector(b: &[u8], enc: VectorEncoding, dims: usize) -> Option<Vec<f32>> {
    if b.len() != dims * enc.width() {
        return None;
    }
    Some(match enc {
        VectorEncoding::F32 => b
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
        VectorEncoding::F16 => b
            .chunks_exact(2)
            .map(|c| f16_to_f32(u16::from_le_bytes([c[0], c[1]])))
            .collect(),
    })
}

/// Parses a JSON array of numbers (`"[0.1, -2, 3e-1]"`).
pub fn parse_vector(text: &str) -> std::result::Result<Vec<f32>, String> {
    let inner = text
        .trim()
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .ok_or("expected a JSON array of numbers")?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    inner
        .split(',')
        .map(|x| {
            x.trim()
                .parse::<f32>()
                .map_err(|_| format!("{:?} is not a number", x.trim()))
        })
        .collect()
}

/// The sign bits of `v`: bit `i` (byte `i / 8`, bit `i % 8`) is set when
/// component `i` is positive.
pub fn sign_bits(v: &[f32]) -> Vec<u8> {
    let mut out = vec![0u8; v.len().div_ceil(8)];
    for (i, x) in v.iter().enumerate() {
        if *x > 0.0 {
            out[i / 8] |= 1 << (i % 8);
        }
    }
    out
}

fn hamming(a: &[u8], b: &[u8]) -> u32 {
    a.iter().zip(b).map(|(x, y)| (x ^ y).count_ones()).sum()
}

/// IEEE 754 single to half precision, rounding to nearest even; overflow becomes
/// infinity (callers only store finite values that fit, or accept the loss).
pub fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x007f_ffff;
    if exp == 0xff {
        // infinity or NaN
        return sign | 0x7c00 | if man != 0 { 0x0200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        // subnormal half (or zero)
        if e < -10 {
            return sign;
        }
        let m = man | 0x0080_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rest = m & ((1 << shift) - 1);
        let mut h = m >> shift;
        if rest > half || (rest == half && h & 1 == 1) {
            h += 1;
        }
        return sign | h as u16;
    }
    let mut h = ((e as u32) << 10) | (man >> 13);
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && h & 1 == 1) {
        h += 1; // may carry into the exponent, which is the correct rounding
    }
    sign | h as u16
}

/// Every half-precision value as single precision, built once: decoding an f16
/// vector is then one table read per component.
fn f16_table() -> &'static [f32] {
    static TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| (0..=u16::MAX).map(f16_to_f32).collect())
}

/// IEEE 754 half to single precision (exact).
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = u32::from(h & 0x8000) << 16;
    let exp = u32::from((h >> 10) & 0x1f);
    let man = u32::from(h & 0x03ff);
    let bits = match (exp, man) {
        (0, 0) => sign,
        (0, m) => {
            // subnormal: normalise
            let mut e = 127 - 15 + 1;
            let mut m = m;
            while m & 0x0400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | ((e as u32) << 23) | ((m & 0x03ff) << 13)
        }
        (0x1f, m) => sign | 0x7f80_0000 | (m << 13),
        (e, m) => sign | ((e + 127 - 15) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}

fn similarity(metric: Metric, q: &[f32], v: &[f32]) -> f64 {
    match metric {
        Metric::Cosine | Metric::Dot => {
            let mut acc = [0f32; 8];
            let mut qc = q.chunks_exact(8);
            let mut vc = v.chunks_exact(8);
            for (a, b) in (&mut qc).zip(&mut vc) {
                for i in 0..8 {
                    acc[i] += a[i] * b[i];
                }
            }
            let tail: f32 = qc
                .remainder()
                .iter()
                .zip(vc.remainder())
                .map(|(a, b)| a * b)
                .sum();
            f64::from(acc.iter().sum::<f32>() + tail)
        }
        Metric::L2 => {
            let mut acc = [0f32; 8];
            let mut qc = q.chunks_exact(8);
            let mut vc = v.chunks_exact(8);
            for (a, b) in (&mut qc).zip(&mut vc) {
                for i in 0..8 {
                    let d = a[i] - b[i];
                    acc[i] += d * d;
                }
            }
            let tail: f32 = qc
                .remainder()
                .iter()
                .zip(vc.remainder())
                .map(|(a, b)| (a - b) * (a - b))
                .sum();
            -f64::from(acc.iter().sum::<f32>() + tail).sqrt()
        }
    }
}

// ---------------------------------------------------------------------------
// Model definitions

/// Reads the definition of `model` from its live `sys:` statements; `None` when it
/// is not defined.
pub fn model_definition(
    exec: &mut dyn Executor,
    model: ObjectId,
    terms: &TermReader,
    use_cache: bool,
) -> Result<Option<EmbeddingModel>> {
    let one = |exec: &mut dyn Executor, p: &str| -> Result<Option<Value>> {
        let Some(p) = TermReader::encode(exec, &Value::iri(p))? else {
            return Ok(None);
        };
        let Some(o) = exec.query_i64(
            "SELECT o FROM triple WHERE s = +?1 AND p = +?2 AND t_ret IS NULL ORDER BY eid LIMIT 1",
            &[SqlValue::Integer(model.raw()), SqlValue::Integer(p.raw())],
        )?
        else {
            return Ok(None);
        };
        terms
            .decode(exec, ObjectId::from_raw(o), use_cache)
            .map(Some)
    };
    let Some(Value::Int(dims)) = one(exec, vocab::SYS_DIMENSIONS)? else {
        return Ok(None);
    };
    let metric = match one(exec, vocab::SYS_METRIC)? {
        Some(Value::Str(s)) => Metric::from_name(&s),
        _ => None,
    };
    let encoding = match one(exec, vocab::SYS_VECTOR_ENCODING)? {
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

fn require_model(
    exec: &mut dyn Executor,
    model: ObjectId,
    terms: &TermReader,
    use_cache: bool,
) -> Result<EmbeddingModel> {
    model_definition(exec, model, terms, use_cache)?.ok_or_else(|| {
        Error::invalid_query(format!(
            "{model} is not a defined embedding model (define it with define_embedding_model)"
        ))
    })
}

// ---------------------------------------------------------------------------
// Search

/// One vector recall. `VectorQuery::default()` has no model; set `model` and
/// `vector`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VectorQuery {
    /// The model IRI's ObjectId.
    pub model: Option<ObjectId>,
    /// The query vector, in the model's dimensions.
    pub vector: Vec<f32>,
    /// At most this many hits. `None` = 10.
    pub limit: Option<usize>,
    /// Only statements that are members of at least one of these graphs, the
    /// membership being visible in the view. `None` = every statement;
    /// `Some(vec![])` matches nothing.
    pub graphs: Option<Vec<ObjectId>>,
    /// Only statements with one of these predicates. `None` = any predicate.
    pub predicates: Option<Vec<ObjectId>>,
    /// Keep the `n` best candidates by Hamming distance on the sign bits, then
    /// re-rank them exactly. `None` = score every visible embedding exactly.
    pub prefilter: Option<usize>,
    /// The predicate read as a confidence layer. `None` = `v:confidence`.
    pub confidence: Option<ObjectId>,
}

impl VectorQuery {
    /// A query for the `limit` nearest statements to `vector` under `model`.
    pub fn new(model: ObjectId, vector: Vec<f32>) -> VectorQuery {
        VectorQuery {
            model: Some(model),
            vector,
            ..VectorQuery::default()
        }
    }
}

/// The default number of hits of a vector query.
pub const DEFAULT_LIMIT: usize = 10;

/// One recalled statement with its similarity and evidence.
///
/// Hits are ordered by [`RANK_POLICY`]: similarity descending, then confidence
/// descending (absent last), confirmations descending, authors descending,
/// `added_at` descending (newer first), and statement eid ascending.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorHit {
    /// The statement.
    pub eid: Eid,
    /// Its subject.
    pub s: ObjectId,
    /// Its predicate.
    pub p: ObjectId,
    /// Its object.
    pub o: ObjectId,
    /// Larger is closer: cosine similarity, dot product, or negated L2 distance.
    pub similarity: f64,
    /// 1-based position under the ranking policy.
    pub rank: u64,
    /// The evidence components, read in the query's view.
    pub evidence: TextEvidence,
}

/// The result of a vector recall.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VectorResult {
    /// The hits, best first.
    pub hits: Vec<VectorHit>,
    /// Visible embeddings considered (before a prefilter cut).
    pub candidates: u64,
    /// True when every visible embedding was scored exactly; false after a
    /// prefilter, whose cut may have dropped a closer statement.
    pub exact: bool,
}

struct Candidate {
    eid: Eid,
    similarity: f64,
}

/// Unpacks a stored vector into `out` (reused across rows); false when the length
/// does not match.
fn decode_into(b: &[u8], enc: VectorEncoding, dims: usize, out: &mut Vec<f32>) -> bool {
    if b.len() != dims * enc.width() {
        return false;
    }
    out.clear();
    match enc {
        VectorEncoding::F32 => out.extend(
            b.chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])),
        ),
        VectorEncoding::F16 => {
            let table = f16_table();
            out.extend(
                b.chunks_exact(2)
                    .map(|c| table[usize::from(u16::from_le_bytes([c[0], c[1]]))]),
            )
        }
    }
    true
}

/// The visible-statement filter shared by search and [`unembedded`]: time
/// predicates on `t`, predicate list, graph membership. `None` when the filter can
/// match nothing.
fn visibility(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    predicates: &Option<Vec<ObjectId>>,
    graphs: &Option<Vec<ObjectId>>,
    params: &mut Params,
) -> Result<Option<Vec<String>>> {
    if matches!(graphs, Some(g) if g.is_empty()) || matches!(predicates, Some(p) if p.is_empty()) {
        return Ok(None);
    }
    let mut conds = Vec::new();
    let time = scan_predicates(spec, "t", params);
    if !time.is_empty() {
        conds.push(time);
    }
    if let Some(ps) = predicates {
        let list: Vec<String> = ps.iter().map(|p| params.push(p.raw())).collect();
        conds.push(format!("t.p IN ({})", list.join(", ")));
    }
    if let Some(gs) = graphs {
        let Some(ig) = TermReader::encode(exec, &Value::iri(vocab::SYS_IN_GRAPH))? else {
            return Ok(None);
        };
        let ig = params.push(ig.raw());
        let list: Vec<String> = gs.iter().map(|g| params.push(g.raw())).collect();
        let mut member = vec![
            "g.s = t.eid".to_string(),
            format!("g.p = {ig}"),
            format!("g.o IN ({})", list.join(", ")),
        ];
        let gt = scan_predicates(spec, "g", params);
        if !gt.is_empty() {
            member.push(gt);
        }
        conds.push(format!(
            "EXISTS (SELECT 1 FROM triple AS g WHERE {})",
            member.join(" AND ")
        ));
    }
    Ok(Some(conds))
}

/// The transaction-time condition on embedding alias `e`: an as-of view sees only
/// the embeddings stored by then, so a past view always answers the same; now and
/// history see every embedding. `None` when there is no condition.
fn embedding_time(spec: &ViewSpec, params: &mut Params) -> Option<String> {
    match spec.tx {
        TxSel::AsOf(TimeRef::Tx(t)) => Some(format!("e.t <= {}", params.push(t as i64))),
        TxSel::AsOf(TimeRef::Instant(ms)) => Some(format!(
            "e.t <= (SELECT coalesce(max(t), 0) FROM tx WHERE instant <= {})",
            params.push(ms)
        )),
        _ => None,
    }
}

/// Recalls the statements selected by `spec` whose embedding under `q.model` is
/// closest to `q.vector`, ranked by [`RANK_POLICY`]. Runs every query on `exec`,
/// in the caller's snapshot (or on the writer of a speculation, which sees its own
/// embeddings). Works on every host: it needs no SQLite extension.
///
/// Polls the operation budget once per scored row and charges the returned hits.
///
/// # Errors
///
/// `InvalidQuery` for a missing or undefined model, a query vector of the wrong
/// dimension, or a non-finite (or, for cosine, zero) query vector.
// @lat: [[query#Vector Recall]]
pub fn search(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    q: &VectorQuery,
    terms: &TermReader,
    use_cache: bool,
) -> Result<VectorResult> {
    let Some(model) = q.model else {
        return Err(Error::invalid_query("a vector query needs a model"));
    };
    let def = require_model(exec, model, terms, use_cache)?;
    let query = def.prepare(&q.vector, "the query vector")?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    let mut params = Params::new();
    let m = params.push(model.raw());
    let Some(mut conds) = visibility(exec, spec, &q.predicates, &q.graphs, &mut params)? else {
        return Ok(VectorResult {
            exact: true,
            ..VectorResult::default()
        });
    };
    conds.insert(0, format!("e.model = +{m}"));
    if let Some(c) = embedding_time(spec, &mut params) {
        conds.push(c);
    }
    let dims = def.dimensions as usize;
    let mut cands: Vec<Candidate> = Vec::new();
    let mut candidates = 0u64;
    let exact = q.prefilter.is_none();
    let column = if exact { "e.vec" } else { "e.bits" };
    // only the eid and one blob per visible embedding: no sort (the cut orders by
    // similarity and eid) and no per-row statement details (read for the hits only)
    let sql = format!(
        "SELECT e.eid, {column} FROM embedding AS e JOIN triple AS t ON t.eid = e.eid \
         WHERE {}",
        conds.join(" AND ")
    );
    let qbits = sign_bits(&query);
    let mut corrupt = None;
    let mut buf = Vec::with_capacity(dims);
    exec.query(&sql, params.values(), &mut |r| {
        budget::poll()?;
        candidates += 1;
        let eid = r[0].as_i64().unwrap_or(0);
        let blob = r[1].as_blob().unwrap_or_default();
        let score = if exact {
            if !decode_into(blob, def.encoding, dims, &mut buf) {
                corrupt.get_or_insert(eid);
                return Ok(());
            }
            similarity(def.metric, &query, &buf)
        } else {
            // fewer differing signs is closer; negate so larger is better
            -f64::from(hamming(blob, &qbits))
        };
        cands.push(Candidate {
            eid: Eid::from_oid(ObjectId::from_raw(eid)).unwrap_or(Eid::new(0)),
            similarity: score,
        });
        Ok(())
    })?;
    if let Some(eid) = corrupt {
        return Err(Error::InvalidTerm {
            position: crate::error::Position::Object,
            reason: format!("the stored vector of statement {eid} does not match its model"),
        });
    }
    if let Some(n) = q.prefilter {
        keep_best(&mut cands, n.max(limit));
        rescore(exec, model, &def, &query, &mut cands)?;
    }
    // keep the top `limit` by similarity, plus ties at the boundary, whose order
    // the evidence decides
    keep_best(&mut cands, limit);
    let rows = statement_rows(exec, &cands)?;
    let ev = EvidenceReader::new(exec, spec, q.confidence)?;
    let mut hits = Vec::with_capacity(cands.len());
    for c in cands {
        budget::check()?;
        let Some(&(s, p, o, t_add, added_at)) = rows.get(&c.eid) else {
            continue;
        };
        let evidence = ev.evidence(exec, c.eid, TxId(t_add as u64), added_at, terms, use_cache)?;
        hits.push(VectorHit {
            eid: c.eid,
            s,
            p,
            o,
            similarity: c.similarity,
            rank: 0,
            evidence,
        });
    }
    hits.sort_by(compare);
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

type StatementRow = (ObjectId, ObjectId, ObjectId, i64, i64);

/// `(s, p, o, t_add, added_at)` of the candidates' statements, in chunked queries.
fn statement_rows(
    exec: &mut dyn Executor,
    cands: &[Candidate],
) -> Result<std::collections::HashMap<Eid, StatementRow>> {
    let mut out = std::collections::HashMap::with_capacity(cands.len());
    for chunk in cands.chunks(500) {
        let list: Vec<String> = (1..=chunk.len()).map(|i| format!("+?{i}")).collect();
        let sql = format!(
            "SELECT eid, s, p, o, t_add, (SELECT instant FROM tx WHERE tx.t = t_add) \
             FROM triple WHERE eid IN ({})",
            list.join(", ")
        );
        let params: Vec<SqlValue> = chunk
            .iter()
            .map(|c| SqlValue::Integer(c.eid.oid().raw()))
            .collect();
        exec.query(&sql, &params, &mut |r| {
            let i = |k: usize| r[k].as_i64().unwrap_or(0);
            if let Some(eid) = Eid::from_oid(ObjectId::from_raw(i(0))) {
                out.insert(
                    eid,
                    (
                        ObjectId::from_raw(i(1)),
                        ObjectId::from_raw(i(2)),
                        ObjectId::from_raw(i(3)),
                        i(4),
                        i(5),
                    ),
                );
            }
            Ok(())
        })?;
    }
    Ok(out)
}

/// Keeps the `n` candidates with the largest similarity, plus every candidate tied
/// with the `n`-th.
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

/// Replaces prefilter scores with exact similarities.
fn rescore(
    exec: &mut dyn Executor,
    model: ObjectId,
    def: &EmbeddingModel,
    query: &[f32],
    cands: &mut [Candidate],
) -> Result<()> {
    let mut by_eid = std::collections::HashMap::with_capacity(cands.len());
    for (i, c) in cands.iter().enumerate() {
        by_eid.insert(c.eid.oid().raw(), i);
    }
    let eids: Vec<i64> = by_eid.keys().copied().collect();
    for chunk in eids.chunks(500) {
        let list: Vec<String> = (2..chunk.len() + 2).map(|i| format!("+?{i}")).collect();
        let sql = format!(
            "SELECT eid, vec FROM embedding WHERE model = +?1 AND eid IN ({})",
            list.join(", ")
        );
        let mut params = vec![SqlValue::Integer(model.raw())];
        params.extend(chunk.iter().map(|e| SqlValue::Integer(*e)));
        let mut buf = Vec::with_capacity(def.dimensions as usize);
        exec.query(&sql, &params, &mut |r| {
            budget::poll()?;
            let eid = r[0].as_i64().unwrap_or(0);
            if let Some(&i) = by_eid.get(&eid) {
                let blob = r[1].as_blob().unwrap_or_default();
                if decode_into(blob, def.encoding, def.dimensions as usize, &mut buf) {
                    cands[i].similarity = similarity(def.metric, query, &buf);
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn compare_evidence(a: &TextEvidence, b: &TextEvidence) -> Ordering {
    match (a.confidence, b.confidence) {
        (Some(x), Some(y)) => y.total_cmp(&x),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
    .then_with(|| b.confirmations.cmp(&a.confirmations))
    .then_with(|| b.authors.cmp(&a.authors))
    .then_with(|| b.added_at.cmp(&a.added_at))
}

/// The order of [`RANK_POLICY`].
pub fn compare(a: &VectorHit, b: &VectorHit) -> Ordering {
    b.similarity
        .total_cmp(&a.similarity)
        .then_with(|| compare_evidence(&a.evidence, &b.evidence))
        .then_with(|| a.eid.cmp(&b.eid))
}

pub(crate) fn compare_by_evidence(a: (&TextEvidence, Eid), b: (&TextEvidence, Eid)) -> Ordering {
    compare_evidence(a.0, b.0).then_with(|| a.1.cmp(&b.1))
}

/// The statements selected by `spec` that have no embedding under `model`, in eid
/// order, at most `limit` (all when `None`): the backlog of an embedding job.
pub fn unembedded(
    exec: &mut dyn Executor,
    spec: &ViewSpec,
    model: ObjectId,
    limit: Option<usize>,
) -> Result<Vec<Eid>> {
    let mut params = Params::new();
    let m = params.push(model.raw());
    let Some(mut conds) = visibility(exec, spec, &None, &None, &mut params)? else {
        return Ok(Vec::new());
    };
    let when = embedding_time(spec, &mut params)
        .map(|c| format!(" AND {c}"))
        .unwrap_or_default();
    conds.push(format!(
        "NOT EXISTS (SELECT 1 FROM embedding AS e WHERE e.model = +{m} AND e.eid = t.eid{when})"
    ));
    let mut sql = format!(
        "SELECT t.eid FROM triple AS t WHERE {} ORDER BY t.eid",
        conds.join(" AND ")
    );
    if let Some(l) = limit {
        let l = params.push(i64::try_from(l).unwrap_or(i64::MAX));
        sql.push_str(&format!(" LIMIT {l}"));
    }
    let mut out = Vec::new();
    exec.query(&sql, params.values(), &mut |r| {
        budget::poll()?;
        if let Some(e) = r[0]
            .as_i64()
            .and_then(|e| Eid::from_oid(ObjectId::from_raw(e)))
        {
            out.push(e);
        }
        Ok(())
    })?;
    if budget::active() {
        budget::charge_rows(out.len() as u64)?;
        budget::charge_bytes(out.len() as u64 * 8)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_round_trips_representable_values_and_rounds_to_nearest_even() {
        for x in [
            0.0f32,
            -0.0,
            1.0,
            -2.5,
            0.333_251_95,
            65504.0,
            6.103_515_6e-5,
        ] {
            assert_eq!(f16_to_f32(f32_to_f16(x)).to_bits(), x.to_bits(), "{x}");
        }
        // smallest subnormal half
        assert_eq!(f16_to_f32(1), 5.960_464_5e-8);
        assert_eq!(f16_to_f32(f32_to_f16(5.960_464_5e-8)), 5.960_464_5e-8);
        assert_eq!(f32_to_f16(1e6), 0x7c00); // overflow
        assert_eq!(f32_to_f16(1e-9), 0); // underflow
                                         // 1 + 2^-11 lies halfway between 1 and 1 + 2^-10: ties to even (1)
        assert_eq!(f32_to_f16(1.0 + 2f32.powi(-11)), 0x3c00);
        // 1 + 3·2^-11 lies halfway between 1 + 2^-10 and 1 + 2^-9: ties to even
        assert_eq!(f32_to_f16(1.0 + 3.0 * 2f32.powi(-11)), 0x3c02);
        for i in 0..2000 {
            let x = (i as f32 - 1000.0) * 0.0137;
            let back = f16_to_f32(f32_to_f16(x));
            assert!((back - x).abs() <= x.abs() * 1e-3 + 1e-4, "{x} -> {back}");
        }
    }

    #[test]
    fn vectors_pack_and_unpack() {
        let v = vec![0.5f32, -1.25, 3.0];
        for enc in [VectorEncoding::F32, VectorEncoding::F16] {
            let b = encode_vector(&v, enc);
            assert_eq!(b.len(), 3 * enc.width());
            assert_eq!(decode_vector(&b, enc, 3).unwrap(), v);
            assert!(decode_vector(&b, enc, 4).is_none());
        }
        assert_eq!(
            sign_bits(&[1.0, -1.0, 0.0, 2.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
            vec![0b1111_1001, 1]
        );
    }

    #[test]
    fn similarities_are_larger_when_closer() {
        let a = [1.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let b = [0.0f32, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        assert_eq!(similarity(Metric::Cosine, &a, &a), 1.0);
        assert_eq!(similarity(Metric::Cosine, &a, &b), 0.0);
        assert_eq!(similarity(Metric::L2, &a, &a), 0.0);
        assert!((similarity(Metric::L2, &a, &b) + 2f64.sqrt()).abs() < 1e-6);
        let c = [2.0f32; 10];
        assert_eq!(similarity(Metric::Dot, &c, &c), 40.0);
    }

    #[test]
    fn models_check_vectors() {
        let m = EmbeddingModel::new(3);
        assert!(m.prepare(&[1.0, 2.0], "v").is_err());
        assert!(m.prepare(&[1.0, f32::NAN, 0.0], "v").is_err());
        assert!(m.prepare(&[0.0, 0.0, 0.0], "v").is_err());
        let u = m.prepare(&[3.0, 0.0, 4.0], "v").unwrap();
        assert!((u[0] - 0.6).abs() < 1e-6 && (u[2] - 0.8).abs() < 1e-6);
        let dot = EmbeddingModel {
            metric: Metric::Dot,
            ..m
        };
        assert_eq!(dot.prepare(&[0.0, 0.0, 0.0], "v").unwrap(), vec![0.0; 3]);
        assert!(EmbeddingModel::new(0).check().is_err());
        for n in ["cosine", "dot", "l2"] {
            assert_eq!(Metric::from_name(n).unwrap().name(), n);
        }
        for n in ["f32", "f16"] {
            assert_eq!(VectorEncoding::from_name(n).unwrap().name(), n);
        }
    }
}
