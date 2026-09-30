//! Importing a fact bundle: idempotent assert of every statement (spec `fact-bundles`).

use std::collections::HashMap;

use super::Tx;
use crate::bundle::{BTerm, Bundle, ImportReport, ImportedStatement};
use crate::error::{Error, Position, Result};
use crate::id::{Eid, ObjectId};
use crate::report::{AssertOpts, OnExisting};
use crate::value::Value;
use crate::vocab;

impl Tx<'_> {
    /// Writes `bundle` into this transaction and maps every bundle-local id to an
    /// eid. Statements are written references first, each through the assert
    /// pipeline: a `sys:inGraph` membership with [`Tx::add_to_graph`], every other
    /// statement with [`Tx::assert`] and its valid time. Anonymous node labels get
    /// fresh `NODE` ids, one per label.
    ///
    /// Assert is idempotent, so importing the same bundle again changes nothing
    /// (except that anonymous nodes are minted again, as RDF blank nodes are on a
    /// second load), and importing onto a database that already holds the root fact
    /// maps the root to that eid and attaches the layers to it. Schema checks of
    /// this database apply; any failure fails the caller's transaction as a whole.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidTerm`] for a malformed bundle ([`Bundle::check`]),
    /// [`Error::Unsupported`] for a bundle whose statements reference each other in
    /// a cycle (e7 about e8 about e7 cannot be asserted idempotently), both before
    /// anything is written, and the errors of assert (`UniqueViolation`,
    /// `ValueTypeMismatch`, `ReservedNamespace`, ...).
    ///
    /// # Example
    ///
    /// ```
    /// use tm_core::{read, vocab::v, Store, StoreOptions, TxOptions, Valid, Value, ViewSpec};
    /// use tm_rusqlite as host;
    ///
    /// # let dir = tempfile::tempdir().unwrap();
    /// let mut a = Store::open(&host::RusqliteHost::new(), &dir.path().join("a"), StoreOptions::default())?;
    /// let mut b = Store::open(&host::RusqliteHost::new(), &dir.path().join("b"), StoreOptions::default())?;
    /// let r = a.transact(TxOptions::default(), |tx| {
    ///     let job = tx.assert(Value::iri(v("alice")), Value::iri(v("worksAt")), Value::iri(v("acme")), Valid::ALWAYS)?.eid();
    ///     tx.assert(job, Value::iri(v("source")), Value::str("chat"), Valid::ALWAYS)?;
    ///     Ok(())
    /// })?;
    /// let bundle = a.read(|e| read::bundle(e, &ViewSpec::now(), r.asserted[0]))?;
    ///
    /// let mut imported = None;
    /// b.transact(TxOptions::default(), |tx| {
    ///     imported = Some(tx.import_bundle(&bundle)?);
    ///     Ok(())
    /// })?;
    /// let imported = imported.unwrap();
    /// assert!(imported.statements.iter().all(|s| s.new));
    ///
    /// // a second import finds everything in place
    /// let again = b.transact(TxOptions::default(), |tx| tx.import_bundle(&bundle).map(|_| ()))?;
    /// assert!(again.asserted.is_empty());
    /// # Ok::<(), tm_core::Error>(())
    /// ```
    // @lat: [[data-model#Fact Bundles]]
    pub fn import_bundle(&mut self, bundle: &Bundle) -> Result<ImportReport> {
        bundle.check()?;
        let order = bundle
            .import_order()
            .ok_or_else(|| Error::unsupported("bundle with a reference cycle"))?;
        let mut eids: HashMap<u32, (Eid, bool)> = HashMap::new();
        let mut nodes: HashMap<u32, ObjectId> = HashMap::new();
        for i in order {
            let st = &bundle.statements[i];
            let s = self.bundle_term(&st.s, &eids, &mut nodes)?;
            let o = self.bundle_term(&st.o, &eids, &mut nodes)?;
            let opts = AssertOpts {
                valid: st.valid,
                on_existing: OnExisting::Return,
            };
            let done = if st.p == Value::iri(vocab::SYS_IN_GRAPH) {
                let member = Eid::from_oid(s).ok_or_else(|| Error::InvalidTerm {
                    position: Position::Subject,
                    reason: format!("the member of membership {} is not a statement", st.local),
                })?;
                self.add_to_graph(member, o, opts)?
            } else {
                let a = self.assert_with(s, &st.p, o, opts)?;
                (a.eid(), a.is_new())
            };
            eids.insert(st.local, done);
        }
        let statements: Vec<ImportedStatement> = bundle
            .statements
            .iter()
            .map(|st| {
                let (eid, new) = eids[&st.local];
                ImportedStatement {
                    local: st.local,
                    eid,
                    new,
                }
            })
            .collect();
        Ok(ImportReport {
            root: eids[&bundle.root].0,
            statements,
        })
    }

    /// The id of one bundle position: a value encoded here, an already imported
    /// statement, or the node minted for an anonymous label.
    fn bundle_term(
        &mut self,
        t: &BTerm,
        eids: &HashMap<u32, (Eid, bool)>,
        nodes: &mut HashMap<u32, ObjectId>,
    ) -> Result<ObjectId> {
        Ok(match t {
            BTerm::Value(v) => self.encode(v)?,
            // the import order puts references first
            BTerm::Stmt(r) => eids[r].0.oid(),
            BTerm::Node(n) => match nodes.get(n) {
                Some(id) => *id,
                None => {
                    let id = self.new_node()?;
                    nodes.insert(*n, id);
                    id
                }
            },
        })
    }
}
