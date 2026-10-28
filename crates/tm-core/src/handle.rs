//! The backend-neutral write handle.
//!
//! [`TxHandle`] is what a transaction body receives from the facade, whatever
//! engine runs the transaction: the SQL engine of this crate ([`crate::Tx`]) or
//! the storage-trait engine of `tm-store`. Both implement [`TxOps`], an
//! object-safe mirror of their write API, so one closure type serves every
//! backend and the query front ends write through the same handle.

use std::any::Any;
use std::sync::Arc;

use crate::bundle::{Bundle, ImportReport};
use crate::engine::{PredicateSchema, Tx};
use crate::error::{Error, Result};
use crate::exec::Executor;
use crate::id::{Eid, ObjectId, TxId};
use crate::report::{AssertOpts, Asserted, IdUsage, OnExisting, Patch, TxOptions, TxReport, Valid};
use crate::store::ReadStore;
use crate::value::Value;
use crate::vector::EmbeddingModel;

#[doc(hidden)]
pub use crate::{
    bundle::Bundle as __Bundle, bundle::ImportReport as __ImportReport,
    engine::PredicateSchema as __PredicateSchema, error::Result as __Result, id::Eid as __Eid,
    id::ObjectId as __ObjectId, id::TxId as __TxId, report::AssertOpts as __AssertOpts,
    report::Asserted as __Asserted, report::IdUsage as __IdUsage, report::Patch as __Patch,
    report::TxOptions as __TxOptions, report::TxReport as __TxReport, report::Valid as __Valid,
    value::Value as __Value, vector::EmbeddingModel as __EmbeddingModel, Obj as __Obj,
};

/// A statement position: an encoded id or a value to encode.
#[derive(Clone, Debug, PartialEq)]
pub enum Obj {
    /// An ObjectId (also an eid or a transaction id).
    Id(ObjectId),
    /// A value, interned on the write path.
    Val(Value),
}

/// Values accepted in statement positions of a [`TxHandle`]: ids or fresh values.
pub trait IntoObj {
    /// The position.
    fn into_obj(self) -> Obj;
}

impl IntoObj for Obj {
    fn into_obj(self) -> Obj {
        self
    }
}

impl IntoObj for ObjectId {
    fn into_obj(self) -> Obj {
        Obj::Id(self)
    }
}

impl IntoObj for Eid {
    fn into_obj(self) -> Obj {
        Obj::Id(self.oid())
    }
}

impl IntoObj for TxId {
    fn into_obj(self) -> Obj {
        Obj::Id(self.oid())
    }
}

impl IntoObj for Value {
    fn into_obj(self) -> Obj {
        Obj::Val(self)
    }
}

impl IntoObj for &Value {
    fn into_obj(self) -> Obj {
        Obj::Val(self.clone())
    }
}

impl crate::engine::IntoObject for Obj {
    fn into_object(self, tx: &mut Tx<'_>) -> Result<ObjectId> {
        match self {
            Obj::Id(id) => id.into_object(tx),
            Obj::Val(v) => v.into_object(tx),
        }
    }
}

/// The write API of a transaction engine, object-safe.
#[allow(missing_docs)]
pub trait TxOps {
    fn assert_with(&mut self, s: Obj, p: Obj, o: Obj, opts: AssertOpts) -> Result<Asserted>;
    fn create(&mut self, s: Obj, p: Obj, o: Obj, valid: Valid) -> Result<Eid>;
    fn meta(&mut self, p: Obj, o: Obj) -> Result<Eid>;
    fn confirm(&mut self, eid: Eid) -> Result<Eid>;
    fn upsert(&mut self, p: Obj, o: Obj) -> Result<ObjectId>;
    fn derive(&mut self, s: Obj, p: Obj, o: Obj, valid: Valid, support: &[Eid])
        -> Result<Asserted>;
    fn upsert_derived(&mut self, p: Obj, o: Obj, support: &[Eid]) -> Result<ObjectId>;
    fn retract(&mut self, eid: Eid) -> Result<bool>;
    fn retract_matching(
        &mut self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<Eid>>;
    fn supersede(&mut self, root: Eid, patch: Patch) -> Result<Eid>;
    fn add_to_graph(&mut self, eid: Eid, graph: Obj, opts: AssertOpts) -> Result<(Eid, bool)>;
    fn remove_from_graph(&mut self, eid: Eid, graph: Obj) -> Result<bool>;
    fn clear_graph(&mut self, graph: Obj) -> Result<Vec<Eid>>;
    fn create_graph(&mut self, graph: Obj) -> Result<Asserted>;
    fn drop_graph(&mut self, graph: Obj) -> Result<Vec<Eid>>;
    fn graph_declared(&mut self, graph: Obj) -> Result<bool>;
    fn graph_has_members(&mut self, graph: Obj) -> Result<bool>;
    fn live_graphs(&mut self) -> Result<Vec<ObjectId>>;
    fn schema(&mut self, p: ObjectId) -> Result<PredicateSchema>;
    fn set_vocab(&mut self, iri: &str) -> Result<()>;
    fn set_prefix(&mut self, name: &str, iri: &str) -> Result<()>;
    fn set_volatile(&mut self, s: Obj, key: Obj, value: Obj) -> Result<()>;
    fn clear_volatile(&mut self, s: Obj, key: Obj) -> Result<()>;
    fn import_bundle(&mut self, bundle: &Bundle) -> Result<ImportReport>;
    fn embedding_model(&mut self, model: Obj) -> Result<Option<EmbeddingModel>>;
    fn define_embedding_model(&mut self, model: Obj, def: &EmbeddingModel) -> Result<bool>;
    fn drop_embedding_model(&mut self, model: Obj) -> Result<bool>;
    fn set_embedding(&mut self, eid: Eid, model: Obj, vector: &[f32]) -> Result<bool>;
    fn t(&self) -> TxId;
    fn instant(&self) -> i64;
    fn id_usage(&self) -> IdUsage;
    fn options(&self) -> TxOptions;
    fn report(&self) -> TxReport;
    fn encode(&mut self, v: Obj) -> Result<ObjectId>;
    fn lookup(&mut self, v: &Value) -> Result<Option<ObjectId>>;
    fn decode(&mut self, id: ObjectId) -> Result<Value>;
    fn new_node(&mut self) -> Result<ObjectId>;
    fn new_bnode(&mut self) -> Result<ObjectId>;
    /// Runs `f` on the SQL connection of the transaction; `MissingCapability`
    /// on an engine without one.
    fn read_exec(&mut self, f: &mut dyn FnMut(&mut dyn Executor) -> Result<()>) -> Result<()>;
    /// Runs `f` on the storage-trait state of the transaction; `None` on an
    /// engine without one (the SQL engine).
    fn read_store(&mut self, f: &mut dyn FnMut(&dyn ReadStore) -> Result<()>)
        -> Option<Result<()>>;
}

/// Implements the forwarding part of [`TxOps`] for an engine `Tx` type whose
/// inherent methods have the write API's names and accept [`$crate::handle::__Obj`] positions.
#[macro_export]
#[doc(hidden)]
macro_rules! forward_tx_ops {
    () => {
        fn assert_with(
            &mut self,
            s: $crate::handle::__Obj,
            p: $crate::handle::__Obj,
            o: $crate::handle::__Obj,
            opts: $crate::handle::__AssertOpts,
        ) -> $crate::handle::__Result<$crate::handle::__Asserted> {
            Self::assert_with(self, s, p, o, opts)
        }
        fn create(
            &mut self,
            s: $crate::handle::__Obj,
            p: $crate::handle::__Obj,
            o: $crate::handle::__Obj,
            valid: $crate::handle::__Valid,
        ) -> $crate::handle::__Result<$crate::handle::__Eid> {
            Self::create(self, s, p, o, valid)
        }
        fn meta(
            &mut self,
            p: $crate::handle::__Obj,
            o: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<$crate::handle::__Eid> {
            Self::meta(self, p, o)
        }
        fn confirm(
            &mut self,
            eid: $crate::handle::__Eid,
        ) -> $crate::handle::__Result<$crate::handle::__Eid> {
            Self::confirm(self, eid)
        }
        fn upsert(
            &mut self,
            p: $crate::handle::__Obj,
            o: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<$crate::handle::__ObjectId> {
            Self::upsert(self, p, o)
        }
        fn derive(
            &mut self,
            s: $crate::handle::__Obj,
            p: $crate::handle::__Obj,
            o: $crate::handle::__Obj,
            valid: $crate::handle::__Valid,
            support: &[$crate::handle::__Eid],
        ) -> $crate::handle::__Result<$crate::handle::__Asserted> {
            Self::derive(self, s, p, o, valid, support)
        }
        fn upsert_derived(
            &mut self,
            p: $crate::handle::__Obj,
            o: $crate::handle::__Obj,
            support: &[$crate::handle::__Eid],
        ) -> $crate::handle::__Result<$crate::handle::__ObjectId> {
            Self::upsert_derived(self, p, o, support)
        }
        fn retract(&mut self, eid: $crate::handle::__Eid) -> $crate::handle::__Result<bool> {
            Self::retract(self, eid)
        }
        fn retract_matching(
            &mut self,
            s: Option<$crate::handle::__ObjectId>,
            p: Option<$crate::handle::__ObjectId>,
            o: Option<$crate::handle::__ObjectId>,
        ) -> $crate::handle::__Result<Vec<$crate::handle::__Eid>> {
            Self::retract_matching(self, s, p, o)
        }
        fn supersede(
            &mut self,
            root: $crate::handle::__Eid,
            patch: $crate::handle::__Patch,
        ) -> $crate::handle::__Result<$crate::handle::__Eid> {
            Self::supersede(self, root, patch)
        }
        fn add_to_graph(
            &mut self,
            eid: $crate::handle::__Eid,
            graph: $crate::handle::__Obj,
            opts: $crate::handle::__AssertOpts,
        ) -> $crate::handle::__Result<($crate::handle::__Eid, bool)> {
            Self::add_to_graph(self, eid, graph, opts)
        }
        fn remove_from_graph(
            &mut self,
            eid: $crate::handle::__Eid,
            graph: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<bool> {
            Self::remove_from_graph(self, eid, graph)
        }
        fn clear_graph(
            &mut self,
            graph: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<Vec<$crate::handle::__Eid>> {
            Self::clear_graph(self, graph)
        }
        fn create_graph(
            &mut self,
            graph: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<$crate::handle::__Asserted> {
            Self::create_graph(self, graph)
        }
        fn drop_graph(
            &mut self,
            graph: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<Vec<$crate::handle::__Eid>> {
            Self::drop_graph(self, graph)
        }
        fn graph_declared(
            &mut self,
            graph: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<bool> {
            Self::graph_declared(self, graph)
        }
        fn graph_has_members(
            &mut self,
            graph: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<bool> {
            Self::graph_has_members(self, graph)
        }
        fn live_graphs(&mut self) -> $crate::handle::__Result<Vec<$crate::handle::__ObjectId>> {
            Self::live_graphs(self)
        }
        fn schema(
            &mut self,
            p: $crate::handle::__ObjectId,
        ) -> $crate::handle::__Result<$crate::handle::__PredicateSchema> {
            Self::schema(self, p)
        }
        fn set_vocab(&mut self, iri: &str) -> $crate::handle::__Result<()> {
            Self::set_vocab(self, iri)
        }
        fn set_prefix(&mut self, name: &str, iri: &str) -> $crate::handle::__Result<()> {
            Self::set_prefix(self, name, iri)
        }
        fn set_volatile(
            &mut self,
            s: $crate::handle::__Obj,
            key: $crate::handle::__Obj,
            value: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<()> {
            Self::set_volatile(self, s, key, value)
        }
        fn clear_volatile(
            &mut self,
            s: $crate::handle::__Obj,
            key: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<()> {
            Self::clear_volatile(self, s, key)
        }
        fn import_bundle(
            &mut self,
            bundle: &$crate::handle::__Bundle,
        ) -> $crate::handle::__Result<$crate::handle::__ImportReport> {
            Self::import_bundle(self, bundle)
        }
        fn embedding_model(
            &mut self,
            model: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<Option<$crate::handle::__EmbeddingModel>> {
            Self::embedding_model(self, model)
        }
        fn define_embedding_model(
            &mut self,
            model: $crate::handle::__Obj,
            def: &$crate::handle::__EmbeddingModel,
        ) -> $crate::handle::__Result<bool> {
            Self::define_embedding_model(self, model, def)
        }
        fn drop_embedding_model(
            &mut self,
            model: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<bool> {
            Self::drop_embedding_model(self, model)
        }
        fn set_embedding(
            &mut self,
            eid: $crate::handle::__Eid,
            model: $crate::handle::__Obj,
            vector: &[f32],
        ) -> $crate::handle::__Result<bool> {
            Self::set_embedding(self, eid, model, vector)
        }
        fn t(&self) -> $crate::handle::__TxId {
            Self::t(self)
        }
        fn instant(&self) -> i64 {
            Self::instant(self)
        }
        fn id_usage(&self) -> $crate::handle::__IdUsage {
            Self::id_usage(self)
        }
        fn options(&self) -> $crate::handle::__TxOptions {
            Self::options(self)
        }
        fn report(&self) -> $crate::handle::__TxReport {
            Self::report(self)
        }
        fn encode(
            &mut self,
            v: $crate::handle::__Obj,
        ) -> $crate::handle::__Result<$crate::handle::__ObjectId> {
            Self::encode(self, v)
        }
        fn lookup(
            &mut self,
            v: &$crate::handle::__Value,
        ) -> $crate::handle::__Result<Option<$crate::handle::__ObjectId>> {
            Self::lookup(self, v)
        }
        fn decode(
            &mut self,
            id: $crate::handle::__ObjectId,
        ) -> $crate::handle::__Result<$crate::handle::__Value> {
            Self::decode(self, id)
        }
        fn new_node(&mut self) -> $crate::handle::__Result<$crate::handle::__ObjectId> {
            Self::new_node(self)
        }
        fn new_bnode(&mut self) -> $crate::handle::__Result<$crate::handle::__ObjectId> {
            Self::new_bnode(self)
        }
    };
}

impl TxOps for Tx<'_> {
    crate::forward_tx_ops!();

    fn read_exec(&mut self, f: &mut dyn FnMut(&mut dyn Executor) -> Result<()>) -> Result<()> {
        Tx::read_with(self, |e| f(e))
    }

    fn read_store(
        &mut self,
        _f: &mut dyn FnMut(&dyn ReadStore) -> Result<()>,
    ) -> Option<Result<()>> {
        None
    }
}

/// The write handle a transaction body receives, on any backend.
///
/// It has the write API of the engine running the transaction: assert, create,
/// retract, supersede, graphs, schema, volatile state, bundles, embeddings. Reads
/// inside the transaction see its own writes.
pub struct TxHandle<'a> {
    ops: &'a mut dyn TxOps,
    ext: Option<Arc<dyn Any + Send + Sync>>,
}

impl std::fmt::Debug for TxHandle<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tx").field("t", &self.ops.t()).finish()
    }
}

impl<'a> TxHandle<'a> {
    /// Wraps an engine transaction.
    pub fn new(ops: &'a mut dyn TxOps) -> TxHandle<'a> {
        TxHandle { ops, ext: None }
    }

    /// Asserts `(s, p, o)` valid over `valid`, idempotently.
    pub fn assert(
        &mut self,
        s: impl IntoObj,
        p: impl IntoObj,
        o: impl IntoObj,
        valid: Valid,
    ) -> Result<Asserted> {
        self.assert_with(
            s,
            p,
            o,
            AssertOpts {
                valid,
                on_existing: OnExisting::Return,
            },
        )
    }

    /// Assert with options.
    pub fn assert_with(
        &mut self,
        s: impl IntoObj,
        p: impl IntoObj,
        o: impl IntoObj,
        opts: AssertOpts,
    ) -> Result<Asserted> {
        self.ops
            .assert_with(s.into_obj(), p.into_obj(), o.into_obj(), opts)
    }

    /// Always inserts a new statement.
    pub fn create(
        &mut self,
        s: impl IntoObj,
        p: impl IntoObj,
        o: impl IntoObj,
        valid: Valid,
    ) -> Result<Eid> {
        self.ops
            .create(s.into_obj(), p.into_obj(), o.into_obj(), valid)
    }

    /// Asserts `(this transaction, p, o)`.
    pub fn meta(&mut self, p: impl IntoObj, o: impl IntoObj) -> Result<Eid> {
        self.ops.meta(p.into_obj(), o.into_obj())
    }

    /// Records that this transaction corroborates live statement `eid`.
    pub fn confirm(&mut self, eid: Eid) -> Result<Eid> {
        self.ops.confirm(eid)
    }

    /// The live subject holding `o` for a unique `p`, or a new node.
    pub fn upsert(&mut self, p: impl IntoObj, o: impl IntoObj) -> Result<ObjectId> {
        self.ops.upsert(p.into_obj(), o.into_obj())
    }

    /// Asserts the belief `(s, p, o)` valid over `valid` as derived from every
    /// statement in `support`: the statement and one engine-owned link
    /// `(D sys:derivedFrom e)` per support. When any support is retracted or
    /// superseded, the belief is retracted with it. Deriving the same content from
    /// the same live support set again returns `Existing`; anything else creates
    /// a new statement, so an independently asserted fact never becomes dependent.
    ///
    /// # Errors
    ///
    /// `InvalidTerm` for an empty `support`, `NotLive` for a support that is not
    /// live, and the errors of [`TxHandle::create`].
    ///
    /// # Example
    ///
    /// ```
    /// use tm_core::{vocab::v, Store, StoreOptions, TxHandle, TxOptions, Valid, Value};
    /// use tm_rusqlite as host;
    ///
    /// # let dir = tempfile::tempdir().unwrap();
    /// # let path = dir.path().join("db");
    /// let mut store = Store::open(&host::RusqliteHost::new(), &path, StoreOptions::default())?;
    /// let r = store.transact(TxOptions::default(), |tx| {
    ///     let mut tx = TxHandle::new(tx);
    ///     let e1 = tx.assert(Value::iri(v("alice")), Value::iri(v("ordered")), Value::iri(v("tea")), Valid::ALWAYS)?.eid();
    ///     let d = tx.derive(Value::iri(v("alice")), Value::iri(v("prefers")), Value::iri(v("tea")), Valid::ALWAYS, &[e1])?;
    ///     assert!(d.is_new());
    ///     // the same content from the same support is the same belief
    ///     let again = tx.derive(Value::iri(v("alice")), Value::iri(v("prefers")), Value::iri(v("tea")), Valid::ALWAYS, &[e1])?;
    ///     assert_eq!(again.eid(), d.eid());
    ///     Ok(())
    /// })?;
    /// assert_eq!(r.asserted.len(), 3); // the evidence, the belief and its link
    /// # Ok::<(), tm_core::Error>(())
    /// ```
    pub fn derive(
        &mut self,
        s: impl IntoObj,
        p: impl IntoObj,
        o: impl IntoObj,
        valid: Valid,
        support: &[Eid],
    ) -> Result<Asserted> {
        self.ops
            .derive(s.into_obj(), p.into_obj(), o.into_obj(), valid, support)
    }

    /// The node `n` of a live key `(n p o)` derived from exactly the live support
    /// set `support`, or a new node whose key is written through
    /// [`TxHandle::derive`]. A rule engine names an invented node this way: a
    /// re-run reuses it, and its key falls with its support.
    ///
    /// # Errors
    ///
    /// `InvalidTerm` for a predicate that is not an IRI, and the errors of
    /// [`TxHandle::derive`].
    pub fn upsert_derived(
        &mut self,
        p: impl IntoObj,
        o: impl IntoObj,
        support: &[Eid],
    ) -> Result<ObjectId> {
        self.ops.upsert_derived(p.into_obj(), o.into_obj(), support)
    }

    /// Retracts a live statement with cascade.
    pub fn retract(&mut self, eid: Eid) -> Result<bool> {
        self.ops.retract(eid)
    }

    /// Retracts every live statement matching the bound positions.
    pub fn retract_matching(
        &mut self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<Eid>> {
        self.ops.retract_matching(s, p, o)
    }

    /// Corrects live statement `root` by cascade-and-replay.
    pub fn supersede(&mut self, root: Eid, patch: Patch) -> Result<Eid> {
        self.ops.supersede(root, patch)
    }

    /// Adds live statement `eid` to `graph`.
    pub fn add_to_graph(
        &mut self,
        eid: Eid,
        graph: impl IntoObj,
        opts: AssertOpts,
    ) -> Result<(Eid, bool)> {
        self.ops.add_to_graph(eid, graph.into_obj(), opts)
    }

    /// Removes statement `eid` from `graph`.
    pub fn remove_from_graph(&mut self, eid: Eid, graph: impl IntoObj) -> Result<bool> {
        self.ops.remove_from_graph(eid, graph.into_obj())
    }

    /// Retracts every live membership in `graph`.
    pub fn clear_graph(&mut self, graph: impl IntoObj) -> Result<Vec<Eid>> {
        self.ops.clear_graph(graph.into_obj())
    }

    /// Declares `graph`.
    pub fn create_graph(&mut self, graph: impl IntoObj) -> Result<Asserted> {
        self.ops.create_graph(graph.into_obj())
    }

    /// Clears `graph` and retracts its declaration.
    pub fn drop_graph(&mut self, graph: impl IntoObj) -> Result<Vec<Eid>> {
        self.ops.drop_graph(graph.into_obj())
    }

    /// Whether `graph` has a live declaration.
    pub fn graph_declared(&mut self, graph: impl IntoObj) -> Result<bool> {
        self.ops.graph_declared(graph.into_obj())
    }

    /// Whether `graph` has a live membership.
    pub fn graph_has_members(&mut self, graph: impl IntoObj) -> Result<bool> {
        self.ops.graph_has_members(graph.into_obj())
    }

    /// Every graph with a live membership or declaration.
    pub fn live_graphs(&mut self) -> Result<Vec<ObjectId>> {
        self.ops.live_graphs()
    }

    /// The schema of `p` at this point of the transaction.
    pub fn schema(&mut self, p: ObjectId) -> Result<PredicateSchema> {
        self.ops.schema(p)
    }

    /// Sets `@vocab`.
    pub fn set_vocab(&mut self, iri: &str) -> Result<()> {
        self.ops.set_vocab(iri)
    }

    /// Declares prefix `name` for `iri`.
    pub fn set_prefix(&mut self, name: &str, iri: &str) -> Result<()> {
        self.ops.set_prefix(name, iri)
    }

    /// Upserts a volatile value.
    pub fn set_volatile(
        &mut self,
        s: impl IntoObj,
        key: impl IntoObj,
        value: impl IntoObj,
    ) -> Result<()> {
        self.ops
            .set_volatile(s.into_obj(), key.into_obj(), value.into_obj())
    }

    /// Removes a volatile value.
    pub fn clear_volatile(&mut self, s: impl IntoObj, key: impl IntoObj) -> Result<()> {
        self.ops.clear_volatile(s.into_obj(), key.into_obj())
    }

    /// Imports a fact bundle.
    pub fn import_bundle(&mut self, bundle: &Bundle) -> Result<ImportReport> {
        self.ops.import_bundle(bundle)
    }

    /// The definition of `model` visible to this transaction.
    pub fn embedding_model(&mut self, model: impl IntoObj) -> Result<Option<EmbeddingModel>> {
        self.ops.embedding_model(model.into_obj())
    }

    /// Defines an embedding model.
    pub fn define_embedding_model(
        &mut self,
        model: impl IntoObj,
        def: &EmbeddingModel,
    ) -> Result<bool> {
        self.ops.define_embedding_model(model.into_obj(), def)
    }

    /// Drops an embedding model.
    pub fn drop_embedding_model(&mut self, model: impl IntoObj) -> Result<bool> {
        self.ops.drop_embedding_model(model.into_obj())
    }

    /// Stores the embedding of live statement `eid` under `model`.
    pub fn set_embedding(&mut self, eid: Eid, model: impl IntoObj, vector: &[f32]) -> Result<bool> {
        self.ops.set_embedding(eid, model.into_obj(), vector)
    }

    /// This transaction's number.
    pub fn t(&self) -> TxId {
        self.ops.t()
    }

    /// This transaction's instant (epoch ms).
    pub fn instant(&self) -> i64 {
        self.ops.instant()
    }

    /// The ids allocated so far.
    pub fn id_usage(&self) -> IdUsage {
        self.ops.id_usage()
    }

    /// The options of this transaction.
    pub fn options(&self) -> TxOptions {
        self.ops.options()
    }

    /// The report of everything done so far.
    pub fn report(&self) -> TxReport {
        self.ops.report()
    }

    /// Encodes a value, interning it if needed.
    pub fn encode(&mut self, v: impl IntoObj) -> Result<ObjectId> {
        self.ops.encode(v.into_obj())
    }

    /// Looks a value up without interning it.
    pub fn lookup(&mut self, v: &Value) -> Result<Option<ObjectId>> {
        self.ops.lookup(v)
    }

    /// Decodes an id, seeing this transaction's own terms.
    pub fn decode(&mut self, id: ObjectId) -> Result<Value> {
        self.ops.decode(id)
    }

    /// Allocates a fresh `NODE` id.
    pub fn new_node(&mut self) -> Result<ObjectId> {
        self.ops.new_node()
    }

    /// Allocates a fresh `BNODE` id.
    pub fn new_bnode(&mut self) -> Result<ObjectId> {
        self.ops.new_bnode()
    }

    /// Runs a SQL read on this transaction's connection (SQLite databases).
    ///
    /// # Errors
    ///
    /// `MissingCapability("sql")` on a backend without SQL (redb).
    pub fn read_with<R>(&mut self, f: impl FnOnce(&mut dyn Executor) -> Result<R>) -> Result<R> {
        let mut f = Some(f);
        let mut out = None;
        self.ops.read_exec(&mut |e| {
            out = Some((f.take().expect("called once"))(e)?);
            Ok(())
        })?;
        out.ok_or_else(|| Error::MissingCapability {
            capability: "sql".to_string(),
        })
    }

    /// Runs a read on this transaction's storage-trait state, or `None` on a
    /// SQL-engine transaction.
    pub fn read_store<R>(
        &mut self,
        f: impl FnOnce(&dyn ReadStore) -> Result<R>,
    ) -> Option<Result<R>> {
        let mut f = Some(f);
        let mut out = None;
        let r = self.ops.read_store(&mut |s| {
            out = Some((f.take().expect("called once"))(s)?);
            Ok(())
        })?;
        Some(r.map(|_| out.expect("ran")))
    }

    /// The stored row of statement `eid`, live or retracted, seen by this
    /// transaction.
    pub fn statement(&mut self, eid: Eid) -> Result<Option<crate::store::StmtRow>> {
        let raw = eid.oid().raw();
        if let Some(r) = self.read_store(|s| s.statement(raw)) {
            return r;
        }
        self.read_with(|e| {
            let store = crate::sqlstore::SqlStore::new(e);
            crate::store::ReadStore::statement(&store, raw)
        })
    }

    /// The live statements matching the bound positions, in eid order.
    pub fn live_rows(
        &mut self,
        s: Option<ObjectId>,
        p: Option<ObjectId>,
        o: Option<ObjectId>,
    ) -> Result<Vec<crate::store::StmtRow>> {
        let pat = crate::store::Pattern::new(
            s.map(ObjectId::raw),
            p.map(ObjectId::raw),
            o.map(ObjectId::raw),
        );
        match self.read_store(|st| st.scan_eid(pat, crate::store::Scope::Live)) {
            Some(r) => r,
            None => self.read_with(|e| {
                crate::sqlstore::SqlStore::new(e).scan_eid(pat, crate::store::Scope::Live)
            }),
        }
    }

    /// The live statements with content `(s, p, o)`, in eid order.
    pub fn live_eids(&mut self, s: ObjectId, p: ObjectId, o: ObjectId) -> Result<Vec<Eid>> {
        let rows = self.live_rows(Some(s), Some(p), Some(o))?;
        Ok(rows
            .into_iter()
            .filter_map(|r| Eid::from_oid(ObjectId::from_raw(r.eid)))
            .collect())
    }

    /// True when this transaction runs on the storage-trait engine.
    pub fn on_store(&mut self) -> bool {
        self.ops.read_store(&mut |_| Ok(())).is_some()
    }

    /// Attaches a host object (the facade attaches its query engine).
    pub fn set_extension(&mut self, ext: Arc<dyn Any + Send + Sync>) {
        self.ext = Some(ext);
    }

    /// The host object of type `T`, if attached.
    pub fn extension<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.ext.clone()?.downcast::<T>().ok()
    }
}
