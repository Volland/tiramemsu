//! A storage-trait snapshot as the planner's and the decoder's data source, and
//! the term rows value operations read (the `term` table plus plan-local terms).

use std::cell::RefCell;
use std::collections::HashMap;

use tm_core::codec::{self, Encoded};
use tm_core::store::ReadStore;
use tm_core::term::Term;
use tm_core::{ObjectId, Result, Value};
use tm_exec::decode::TermLoad;
use tm_exec::plan::PlanSource;

/// [`PlanSource`] on a snapshot.
pub struct StoreSource<'a>(pub &'a dyn ReadStore);

impl PlanSource for StoreSource<'_> {
    fn encode(&mut self, v: &Value) -> Result<Option<ObjectId>> {
        tm_store::dict::encode(self.0, v)
    }

    fn instant_to_t(&mut self, ms: i64) -> Result<u64> {
        Ok(self.0.tx_at_or_before(ms)?.max(0) as u64)
    }

    fn pred_multi(&mut self, p: ObjectId) -> Result<bool> {
        self.0.pred_multi(p.raw())
    }
}

/// [`TermLoad`] on a snapshot.
pub struct StoreLoad<'a>(pub &'a dyn ReadStore);

impl TermLoad for StoreLoad<'_> {
    fn load(&mut self, id: i64) -> Result<Option<Term>> {
        tm_store::dict::load_term(self.0, id)
    }
}

/// The columns of one term row a value operation reads (`lex`, `num`, `lang`, `dt`).
#[derive(Clone, Debug, PartialEq)]
pub struct TermBits {
    /// Lexical form.
    pub lex: String,
    /// Numeric value.
    pub num: Option<f64>,
    /// Language tag.
    pub lang: Option<String>,
    /// Datatype IRI ObjectId raw (stored terms only).
    pub dt: Option<i64>,
}

/// Term rows by dictionary id, from the snapshot and from the plan-local terms
/// of constants missing from the dictionary.
pub struct Terms<'a> {
    store: &'a dyn ReadStore,
    synthetic: HashMap<i64, TermBits>,
    cache: RefCell<HashMap<i64, Option<TermBits>>>,
}

impl<'a> Terms<'a> {
    /// The term source of one query.
    pub fn new(store: &'a dyn ReadStore, synthetic: &HashMap<i64, Value>) -> Terms<'a> {
        let mut syn = HashMap::new();
        for (raw, v) in synthetic {
            if let Encoded::Term(t) = codec::encode(v) {
                syn.insert(
                    ObjectId::from_raw(*raw).unsigned_payload() as i64,
                    TermBits {
                        lex: t.lex,
                        num: t.num,
                        lang: t.lang,
                        dt: None,
                    },
                );
            }
        }
        Terms {
            store,
            synthetic: syn,
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// The row with id `id` of the stored dictionary plus the plan-local terms
    /// (`term_source` of the SQL route).
    pub fn row(&self, id: i64) -> Result<Option<TermBits>> {
        if let Some(t) = self.synthetic.get(&id) {
            return Ok(Some(t.clone()));
        }
        self.stored(id)
    }

    /// The row with id `id` of the stored dictionary only.
    pub fn stored(&self, id: i64) -> Result<Option<TermBits>> {
        if let Some(t) = self.cache.borrow().get(&id) {
            return Ok(t.clone());
        }
        let t = self.store.term(id)?.map(|r| TermBits {
            lex: r.lex,
            num: r.num,
            lang: r.lang,
            dt: r.dt,
        });
        self.cache.borrow_mut().insert(id, t.clone());
        Ok(t)
    }
}
