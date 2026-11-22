//! Transitional modeling on tiramemsu: posits and assertions stored as
//! append-only statements and resolved on read.
//!
//! Model after Lars Rönnbäck, "Modeling Conflicting, Unreliable, and Varying
//! Information" (4th revision, 2018, DOI 10.13140/RG.2.2.34381.49121/1).
//!
//! - A **posit** `[D, v, t]` claims nothing by itself. `D` is a set of
//!   `(thing, role)` pairs with each role at most once, `v` a value and `t` the
//!   appearance time. Its id is a hash of `(D, v, t)`.
//! - An **assertion** `!(P, posit, alpha, T)` is a positor's opinion `alpha` in
//!   `[-1, 1]` on a posit. `T` is the transaction instant of the write. A later
//!   assertion by the same positor on the same posit changes the opinion and
//!   `alpha = 0` retracts it. Nothing is updated or deleted.
//! - [`Transitional::in_effect`] resolves the store at a bitemporal point.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use tiramemsu::{Db, Error as StoreError, SparqlResult, TxOptions, Valid, Value};

const NS: &str = "urn:tr:";
fn iri(local: &str) -> Value {
    Value::iri(format!("{NS}{local}"))
}
fn pred(local: &str) -> Value {
    iri(&format!("p:{local}"))
}

/// Errors from the transitional layer.
#[derive(Debug)]
pub enum TrError {
    /// The underlying store failed.
    Store(StoreError),
    /// The write breaks a rule of the model, so nothing was written.
    Rejected(String),
}
impl fmt::Display for TrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrError::Store(e) => write!(f, "store: {e}"),
            TrError::Rejected(m) => write!(f, "rejected: {m}"),
        }
    }
}
impl std::error::Error for TrError {}
impl From<StoreError> for TrError {
    fn from(e: StoreError) -> Self {
        TrError::Store(e)
    }
}
/// Result alias.
pub type Result<T> = std::result::Result<T, TrError>;

/// A stable 128-bit FNV-1a hash in hex. It stands in for a cryptographic hash.
fn hash128(parts: &[&str]) -> String {
    let mut h = [0xcbf29ce484222325u64, 0x6c62272e07bb0143u64];
    for part in parts {
        for byte in (part.len() as u64)
            .to_le_bytes()
            .iter()
            .chain(part.as_bytes())
        {
            for (i, lane) in h.iter_mut().enumerate() {
                *lane ^= u64::from(*byte) ^ i as u64;
                *lane = lane.wrapping_mul(0x100000001b3);
            }
        }
    }
    format!("{:016x}{:016x}", h[0], h[1])
}

/// A dereferencing set: `(thing, role)` pairs, each role at most once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dset(Vec<(String, String)>);

impl Dset {
    /// Builds a set, sorted by role. A repeated role is rejected.
    pub fn new(members: &[(&str, &str)]) -> Result<Dset> {
        let mut v: Vec<(String, String)> = members
            .iter()
            .map(|(t, r)| (t.to_string(), r.to_string()))
            .collect();
        v.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        if v.is_empty() || v.windows(2).any(|w| w[0].1 == w[1].1) {
            return Err(TrError::Rejected(
                "a dset needs members and each role at most once".into(),
            ));
        }
        Ok(Dset(v))
    }
    fn id(&self) -> String {
        let flat: Vec<String> = self.0.iter().map(|(t, r)| format!("{r}={t}")).collect();
        hash128(&flat.iter().map(String::as_str).collect::<Vec<_>>())
    }
}

/// A posit `[D, v, t]`.
#[derive(Clone, Debug)]
pub struct Posit {
    /// What the posit is about.
    pub dset: Dset,
    /// The value claimed for it.
    pub value: Value,
    /// When the state takes effect in the world (an integer tick or epoch ms).
    pub t_app: i64,
}

impl Posit {
    /// The content-addressed id: a hash of the dset, the value and the time.
    pub fn id(&self) -> String {
        hash128(&[
            &self.dset.id(),
            &self.value.to_string(),
            &self.t_app.to_string(),
        ])
    }
}

/// One assertion as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Assertion {
    /// Sequence number, 1-based, in write order.
    pub seq: i64,
    /// Who holds the opinion.
    pub positor: String,
    /// The posit id.
    pub posit: String,
    /// The reliability in `[-1, 1]`; `0` retracts.
    pub alpha: f64,
    /// The transaction instant at which it was written.
    pub t_assert: i64,
}

/// One step of an atomic write.
#[derive(Clone, Debug)]
pub enum Step {
    /// Make a posit exist. Writing an identical posit again changes nothing.
    Posit(Posit),
    /// A positor's opinion on a posit. Use `0.0` to retract.
    Assert {
        /// Who holds the opinion.
        positor: String,
        /// The posit id.
        posit: String,
        /// The reliability.
        alpha: f64,
    },
}

/// The transitional layer over one database.
pub struct Transitional<'a> {
    db: &'a Db,
}

struct Loaded {
    posits: HashMap<String, (String, String, i64)>, // id -> (dset, value, t_app)
    assertions: Vec<Assertion>,
}

impl<'a> Transitional<'a> {
    /// Wraps `db`.
    pub fn new(db: &'a Db) -> Self {
        Transitional { db }
    }

    fn load(&self) -> Result<Loaded> {
        let view = self.db.now();
        let mut posits = HashMap::new();
        let q = format!(
            "SELECT ?p ?d ?v ?t WHERE {{ ?p <{NS}p:dset> ?d ; <{NS}p:value> ?v ; <{NS}p:appearsAt> ?t }}"
        );
        if let SparqlResult::Solutions(s) = view.sparql(&q)? {
            for r in &s.rows {
                if let (Some(Value::Iri(p)), Some(Value::Iri(d)), Some(v), Some(Value::Int(t))) =
                    (&r[0], &r[1], &r[2], &r[3])
                {
                    let id = p.trim_start_matches(&format!("{NS}posit:")).to_string();
                    let dset = d.trim_start_matches(&format!("{NS}dset:")).to_string();
                    posits.insert(id, (dset, v.to_string(), *t));
                }
            }
        }
        let mut assertions = Vec::new();
        let q = format!(
            "SELECT ?by ?p ?al ?seq ?at WHERE {{ ?a <{NS}p:positor> ?by ; <{NS}p:posit> ?p ; \
             <{NS}p:seq> ?seq ; <{NS}p:alpha> ?al ~ ?r . ?r tm:addedAt ?at }}"
        );
        if let SparqlResult::Solutions(s) = view.sparql(&q)? {
            for r in &s.rows {
                if let (
                    Some(Value::Iri(by)),
                    Some(Value::Iri(p)),
                    Some(Value::Double(al)),
                    Some(Value::Int(seq)),
                    Some(Value::DateTime { ms, .. }),
                ) = (&r[0], &r[1], &r[2], &r[3], &r[4])
                {
                    assertions.push(Assertion {
                        seq: *seq,
                        positor: by.trim_start_matches(&format!("{NS}thing:")).to_string(),
                        posit: p.trim_start_matches(&format!("{NS}posit:")).to_string(),
                        alpha: *al,
                        t_assert: *ms,
                    });
                }
            }
        }
        assertions.sort_by_key(|a| a.seq);
        Ok(Loaded { posits, assertions })
    }

    /// Every assertion ever written, in write order.
    pub fn assertions(&self) -> Result<Vec<Assertion>> {
        Ok(self.load()?.assertions)
    }

    /// Writes the steps in one transaction after validating them in order.
    /// Returns the assertions written. If any step is rejected nothing is written.
    pub fn write(&self, steps: Vec<Step>) -> Result<Vec<Assertion>> {
        let mut state = self.load()?;
        let mut next_seq = state.assertions.last().map_or(1, |a| a.seq + 1);
        let mut new_posits: Vec<Posit> = Vec::new();
        let mut new_asserts: Vec<Assertion> = Vec::new();
        for step in &steps {
            match step {
                Step::Posit(p) => {
                    let id = p.id();
                    if state
                        .posits
                        .insert(id, (p.dset.id(), p.value.to_string(), p.t_app))
                        .is_none()
                    {
                        new_posits.push(p.clone());
                    }
                }
                Step::Assert {
                    positor,
                    posit,
                    alpha,
                } => {
                    if !(-1.0..=1.0).contains(alpha) {
                        return Err(TrError::Rejected(format!(
                            "alpha {alpha} is outside [-1, 1]"
                        )));
                    }
                    if !state.posits.contains_key(posit) {
                        return Err(TrError::Rejected(format!("unknown posit {posit}")));
                    }
                    let a = Assertion {
                        seq: next_seq,
                        positor: positor.clone(),
                        posit: posit.clone(),
                        alpha: *alpha,
                        t_assert: 0, // set by the store
                    };
                    next_seq += 1;
                    state.assertions.push(a.clone());
                    bounded(&state, positor)?;
                    new_asserts.push(a);
                }
            }
        }
        self.db.transact(TxOptions::default(), |tx| {
            for p in &new_posits {
                let pid = iri(&format!("posit:{}", p.id()));
                let did = iri(&format!("dset:{}", p.dset.id()));
                tx.assert(&pid, pred("dset"), &did, Valid::ALWAYS)?;
                tx.assert(&pid, pred("value"), &p.value, Valid::ALWAYS)?;
                tx.assert(
                    &pid,
                    pred("appearsAt"),
                    Value::Int(p.t_app),
                    Valid::ALWAYS,
                )?;
                for (thing, role) in &p.dset.0 {
                    // one member node per (thing, role), shared by every dset using it
                    let m = iri(&format!("member:{}", hash128(&[thing, role])));
                    tx.assert(&did, pred("hasMember"), &m, Valid::ALWAYS)?;
                    tx.assert(
                        &m,
                        pred("thing"),
                        iri(&format!("thing:{thing}")),
                        Valid::ALWAYS,
                    )?;
                    tx.assert(
                        &m,
                        pred("role"),
                        iri(&format!("role:{role}")),
                        Valid::ALWAYS,
                    )?;
                }
            }
            for a in &new_asserts {
                let id = iri(&format!("assertion:{}", a.seq));
                tx.assert(
                    &id,
                    pred("positor"),
                    iri(&format!("thing:{}", a.positor)),
                    Valid::ALWAYS,
                )?;
                tx.assert(
                    &id,
                    pred("posit"),
                    iri(&format!("posit:{}", a.posit)),
                    Valid::ALWAYS,
                )?;
                tx.assert(&id, pred("seq"), Value::Int(a.seq), Valid::ALWAYS)?;
                tx.assert(&id, pred("alpha"), Value::Double(a.alpha), Valid::ALWAYS)?;
            }
            Ok(())
        })?;
        // re-read so the returned assertions carry the instant the store gave them
        let all = self.load()?.assertions;
        Ok(all
            .into_iter()
            .filter(|a| new_asserts.iter().any(|n| n.seq == a.seq))
            .collect())
    }

    /// A correction: retract the old opinion and assert a new posit, atomically.
    pub fn correct(
        &self,
        positor: &str,
        old: &str,
        new: Posit,
        alpha: f64,
    ) -> Result<Vec<Assertion>> {
        let new_id = new.id();
        self.write(vec![
            Step::Assert {
                positor: positor.into(),
                posit: old.into(),
                alpha: 0.0,
            },
            Step::Posit(new),
            Step::Assert {
                positor: positor.into(),
                posit: new_id,
                alpha,
            },
        ])
    }

    /// The assertions in effect at assertion time `t_assert_at` and appearance
    /// time `t_app_at`, in write order.
    pub fn in_effect(&self, t_assert_at: i64, t_app_at: i64) -> Result<Vec<Assertion>> {
        let l = self.load()?;
        Ok(resolve(&l, t_assert_at, t_app_at))
    }
}

/// Every assertion strictly later in write order than `a`, by the same positor
/// on the same posit, with alpha 0 and made no later than `at`.
fn retracted(all: &[Assertion], a: &Assertion, at: i64) -> bool {
    all.iter().any(|r| {
        r.alpha == 0.0
            && r.positor == a.positor
            && r.posit == a.posit
            && r.t_assert <= at
            && (r.t_assert, r.seq) > (a.t_assert, a.seq)
    })
}

fn resolve(l: &Loaded, t_assert_at: i64, t_app_at: i64) -> Vec<Assertion> {
    // 1. assertion time and appearance time cut-offs
    let seen: Vec<&Assertion> = l
        .assertions
        .iter()
        .filter(|a| a.t_assert <= t_assert_at && l.posits[&a.posit].2 <= t_app_at)
        .collect();
    let all_to_cut: Vec<Assertion> = l
        .assertions
        .iter()
        .filter(|a| a.t_assert <= t_assert_at)
        .cloned()
        .collect();
    // 2. drop retractions and retracted opinions, then the latest appearance
    //    time per (positor, dset)
    let live: Vec<&Assertion> = seen
        .into_iter()
        .filter(|a| a.alpha != 0.0 && !retracted(&all_to_cut, a, t_assert_at))
        .collect();
    let mut latest_app: HashMap<(&str, &str), i64> = HashMap::new();
    for a in &live {
        let (dset, _, t) = &l.posits[&a.posit];
        let e = latest_app
            .entry((a.positor.as_str(), dset.as_str()))
            .or_insert(*t);
        *e = (*e).max(*t);
    }
    let kept = live.into_iter().filter(|a| {
        let (dset, _, t) = &l.posits[&a.posit];
        latest_app[&(a.positor.as_str(), dset.as_str())] == *t
    });
    // 3. the latest assertion time per (positor, dset, value)
    let mut best: BTreeMap<(String, String, String), &Assertion> = BTreeMap::new();
    for a in kept {
        let (dset, value, _) = &l.posits[&a.posit];
        let key = (a.positor.clone(), dset.clone(), value.clone());
        match best.get(&key) {
            Some(b) if (b.t_assert, b.seq) >= (a.t_assert, a.seq) => {}
            _ => {
                best.insert(key, a);
            }
        }
    }
    let mut out: Vec<Assertion> = best.into_values().cloned().collect();
    out.sort_by_key(|a| a.seq);
    out
}

/// The paper's boundary rule, applied to one positor's live opinions on the
/// alternatives for the same dset and appearance time: the sum over those
/// opinions of `(1 - a/|a|)/2 + a` must not exceed 1.
fn bounded(state: &Loaded, positor: &str) -> Result<()> {
    let mut latest: HashMap<&str, &Assertion> = HashMap::new();
    for a in state.assertions.iter().filter(|a| a.positor == positor) {
        latest.insert(a.posit.as_str(), a); // write order: the last one wins
    }
    let mut sums: HashMap<(&str, i64), f64> = HashMap::new();
    for a in latest.values().filter(|a| a.alpha != 0.0) {
        let (dset, _, t) = &state.posits[&a.posit];
        let term = 0.5 * (1.0 - a.alpha / a.alpha.abs()) + a.alpha;
        *sums.entry((dset.as_str(), *t)).or_insert(0.0) += term;
    }
    match sums.values().find(|s| **s > 1.0 + 1e-9) {
        Some(s) => Err(TrError::Rejected(format!(
            "{positor}'s opinions on alternatives sum to {s}, above 1"
        ))),
        None => Ok(()),
    }
}
