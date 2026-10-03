//! The JSON bridge of the tiramemsu bindings.
//!
//! [`Database::call`] takes an operation name and a JSON argument object and returns
//! a JSON result, so the Node.js and Python crates are a few lines each and behave
//! identically. The wrappers (`bindings/node/lib`, `bindings/python/python`) give the
//! calls typed, idiomatic names.
//!
//! Terms, results and errors use one set of JSON forms, described in [`value`]. A view
//! is `{"kind": "now" | "asOf" | "history", "tx": n, "instant": ms, "validAt": ms}`,
//! where `asOf` takes `tx` or `instant`, and `validAt` may accompany any kind.
//!
//! Reads, `transact` and `cypherWrite` take an optional budget,
//! `{"timeoutMs", "readerTimeoutMs", "maxRows", "maxBytes", "cancelKey"}`, which
//! bounds the call like [`tiramemsu::QueryBudget`]; the `cancel` operation
//! (`{"key"}`) stops a call running with that `cancelKey` from another thread.
//!
//! Bulk imports are sessions held by the bridge: `importBegin` returns
//! `{"session": n}`, `importChunk` (`session`, `ops`, `options`, `budget`) commits
//! one chunk like `transact`, and `importProgress`, `importFinish` and
//! `importCancel` take the `session`. See [`tiramemsu::BulkImport`].
//!
//! Conflict review: the read `conflicts` lists disagreeing values with their
//! evidence, and `previewBundle` (`bundle`) reports what importing a bundle
//! would do without committing; applying it is the `importBundle` op of
//! `transact`, which validates again.
//!
//! Saved answers (`saveAnswer`, `savedAnswer`, `savedAnswers`,
//! `checkSavedAnswers`, `refreshAnswer`, `deleteSavedAnswer`) store a query with
//! its result and report later events as `recheck` or `stale` marks. See
//! [`tiramemsu::SavedAnswer`].
//!
// @lat: [[api#Bindings]]

mod read;
mod saved;
mod tx;
pub mod value;

use serde_json::{json, Value as J};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tiramemsu::{BulkImport, CancelToken, Db, Error, OpenOptions, QueryBudget, TxOptions};

pub use value::{value_from_json, value_to_json};

/// A failed call: a database error with its code, or a rejected argument.
#[derive(Debug)]
pub enum BindError {
    /// The database refused the operation.
    Db(Error),
    /// The arguments were malformed (an unknown operation, a term of the wrong shape, ...).
    Arg(String),
}

/// The result of a bridge call.
pub type Res<T> = std::result::Result<T, BindError>;

pub(crate) fn arg(msg: impl Into<String>) -> BindError {
    BindError::Arg(msg.into())
}

impl From<Error> for BindError {
    fn from(e: Error) -> BindError {
        if let Error::Custom(inner) = &e {
            if let Some(a) = inner.downcast_ref::<value::ArgError>() {
                return BindError::Arg(a.0.clone());
            }
        }
        BindError::Db(e)
    }
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::Db(e) => e.fmt(f),
            BindError::Arg(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for BindError {}

impl BindError {
    /// The error code: the name of the [`Error`] variant, or `InvalidArgument`.
    pub fn code(&self) -> &'static str {
        match self {
            BindError::Arg(_) => "InvalidArgument",
            BindError::Db(e) => match e {
                Error::UniqueViolation { .. } => "UniqueViolation",
                Error::ValueTypeMismatch { .. } => "ValueTypeMismatch",
                Error::SubjectTypeMismatch { .. } => "SubjectTypeMismatch",
                Error::CascadeLimitExceeded { .. } => "CascadeLimitExceeded",
                Error::PathLimitExceeded { .. } => "PathLimitExceeded",
                Error::NotLive(_) => "NotLive",
                Error::InvalidPatch(_) => "InvalidPatch",
                Error::SelfReference(_) => "SelfReference",
                Error::ReservedNamespace(_) => "ReservedNamespace",
                Error::InvalidGraphName { .. } => "InvalidGraphName",
                Error::GraphNotFound { .. } => "GraphNotFound",
                Error::GraphExists { .. } => "GraphExists",
                Error::SchemaConflict { .. } => "SchemaConflict",
                Error::FormatVersion { .. } => "FormatVersion",
                Error::Parse { .. } => "Parse",
                Error::Unsupported { .. } => "Unsupported",
                Error::Eval { .. } => "Eval",
                Error::DeleteConnectedNode { .. } => "DeleteConnectedNode",
                Error::InvalidQuery { .. } => "InvalidQuery",
                Error::MissingCapability { .. } => "MissingCapability",
                Error::InvalidTerm { .. } => "InvalidTerm",
                Error::InvalidInterval { .. } => "InvalidInterval",
                Error::NotUniquePredicate(_) => "NotUniquePredicate",
                Error::IdSpaceExhausted { .. } => "IdSpaceExhausted",
                Error::Reentrant => "Reentrant",
                Error::ImportInProgress => "ImportInProgress",
                Error::TextIndexUnavailable { .. } => "TextIndexUnavailable",
                Error::SavedAnswerNotFound { .. } => "SavedAnswerNotFound",
                Error::Cancelled => "Cancelled",
                Error::DeadlineExceeded { .. } => "DeadlineExceeded",
                Error::PoolTimeout { .. } => "PoolTimeout",
                Error::ResultLimitExceeded { .. } => "ResultLimitExceeded",
                Error::ForeignFile(_) => "ForeignFile",
                Error::Sqlite(_) => "Sqlite",
                Error::Custom(_) => "Custom",
                _ => "Error",
            },
        }
    }

    /// The error as `{"code", "message"}`, the text a wrapper parses back into an exception.
    pub fn to_json(&self) -> J {
        json!({ "code": self.code(), "message": self.to_string() })
    }
}

/// An open database. Calls may come from several threads; reads run in parallel and
/// writes serialize on the single writer.
#[derive(Debug)]
pub struct Database {
    db: Arc<Db>,
    /// The cancellation tokens of calls running with a `cancelKey`, and of keys
    /// cancelled before their call started.
    cancels: Mutex<HashMap<String, CancelToken>>,
    /// Open bulk import sessions by id. A finished or cancelled session is `None`
    /// until its entry is removed.
    imports: Mutex<HashMap<u64, Session>>,
    next_session: AtomicU64,
}

/// One bulk import session; chunks of one session run one at a time.
type Session = Arc<Mutex<Option<BulkImport<'static>>>>;

/// Removes a call's `cancelKey` when the call ends.
struct CancelEntry<'a> {
    db: &'a Database,
    key: Option<String>,
}

impl Drop for CancelEntry<'_> {
    fn drop(&mut self) {
        if let Some(k) = &self.key {
            self.db.cancel_map().remove(k);
        }
    }
}

impl Database {
    /// Opens (creating if needed) the database file at `path`. `options` may carry
    /// `readers`, `busyTimeoutMs`, `termCacheCapacity`, `optimizeEvery`,
    /// `pathMaxHops`, `pathMaxStates`, `readerTimeoutMs`, `textIndex` (a
    /// boolean: build the text index at open), `lftj` (a boolean: route pure cyclic
    /// patterns to the native cyclic-join operator) and `lftjMinRows` (its
    /// estimate threshold); anything else is rejected.
    pub fn open(path: &str, options: &J) -> Res<Database> {
        let opts = Database::open_options(options)?;
        Ok(Database::from_db(Db::open(path, opts)?))
    }

    /// The [`OpenOptions`] of a JSON `options` object, as [`Database::open`] reads
    /// them, for a wrapper that opens the [`Db`] itself (on another host, such as
    /// the WASM one) and then calls [`Database::from_db`].
    pub fn open_options(options: &J) -> Res<OpenOptions> {
        let mut opts = OpenOptions::default();
        if let Some(o) = options.as_object() {
            for (k, v) in o {
                if k == "textIndex" {
                    opts.text_index = v
                        .as_bool()
                        .ok_or_else(|| arg("option textIndex must be a boolean"))?;
                    continue;
                }
                if k == "lftj" {
                    opts.planner.lftj.enabled = v
                        .as_bool()
                        .ok_or_else(|| arg("option lftj must be a boolean"))?;
                    continue;
                }
                let n = v
                    .as_u64()
                    .ok_or_else(|| arg(format!("option {k} must be a non-negative integer")))?;
                match k.as_str() {
                    "readers" => opts.readers = n as usize,
                    "busyTimeoutMs" => opts.busy_timeout = Duration::from_millis(n),
                    "termCacheCapacity" => opts.term_cache_capacity = n as usize,
                    "optimizeEvery" => opts.optimize_every = n,
                    "pathMaxHops" => opts.path_max_hops = n as u32,
                    "pathMaxStates" => opts.path_max_states = n as usize,
                    "readerTimeoutMs" => opts.reader_timeout = Some(Duration::from_millis(n)),
                    "lftjMinRows" => opts.planner.lftj.min_rows_estimate = n,
                    other => return Err(arg(format!("unknown option {other:?}"))),
                }
            }
        } else if !options.is_null() {
            return Err(arg("options must be an object"));
        }
        Ok(opts)
    }

    /// Serves an already open database through the bridge.
    pub fn from_db(db: Db) -> Database {
        Database {
            db: Arc::new(db),
            cancels: Mutex::new(HashMap::new()),
            imports: Mutex::new(HashMap::new()),
            next_session: AtomicU64::new(1),
        }
    }

    fn import_map(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Session>> {
        self.imports.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The session named by `args.session`; with `remove`, it is taken out of the
    /// map (finish and cancel).
    fn session(&self, args: &J, remove: bool) -> Res<Session> {
        let id = args
            .get("session")
            .and_then(J::as_u64)
            .ok_or_else(|| arg("`session` must be a non-negative integer"))?;
        let mut map = self.import_map();
        let s = if remove {
            map.remove(&id)
        } else {
            map.get(&id).cloned()
        };
        s.ok_or_else(|| arg(format!("unknown import session {id}")))
    }

    /// Runs one bulk import operation.
    fn import(&self, op: &str, args: &J) -> Res<J> {
        let lock = |s: &Session| s.lock().unwrap_or_else(|p| p.into_inner()).take();
        match op {
            "importBegin" => {
                let session = self.db.bulk_import_shared()?;
                let id = self.next_session.fetch_add(1, Ordering::Relaxed);
                self.import_map()
                    .insert(id, Arc::new(Mutex::new(Some(session))));
                Ok(json!({ "session": id }))
            }
            "importChunk" => {
                let (budget, _entry) = self.budget(args.get("budget"))?;
                let s = self.session(args, false)?;
                let mut guard = s.lock().unwrap_or_else(|p| p.into_inner());
                let session = guard.as_mut().ok_or_else(|| arg("import session ended"))?;
                let mut out = tx::import_chunk(session, args, budget.as_ref())?;
                out["progress"] = tx::progress_json(session.progress());
                Ok(out)
            }
            "importProgress" => {
                let s = self.session(args, false)?;
                let guard = s.lock().unwrap_or_else(|p| p.into_inner());
                let session = guard.as_ref().ok_or_else(|| arg("import session ended"))?;
                Ok(tx::progress_json(session.progress()))
            }
            "importFinish" => {
                let s = self.session(args, true)?;
                let session = lock(&s).ok_or_else(|| arg("import session ended"))?;
                let summary = session.finish();
                Ok(json!({
                    "progress": tx::progress_json(&summary.progress),
                    "analyzed": summary.analyzed,
                    "statisticsDue": summary.statistics_due,
                    "maintenanceError": summary
                        .maintenance_error
                        .map_or(J::Null, |e| BindError::Db(e).to_json()),
                }))
            }
            "importCancel" => {
                let s = self.session(args, true)?;
                let session = lock(&s).ok_or_else(|| arg("import session ended"))?;
                Ok(tx::progress_json(&session.cancel()))
            }
            other => Err(arg(format!("unknown operation {other:?}"))),
        }
    }

    fn cancel_map(&self) -> std::sync::MutexGuard<'_, HashMap<String, CancelToken>> {
        self.cancels.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The budget of a call, from `{"timeoutMs", "readerTimeoutMs", "maxRows",
    /// "maxBytes", "cancelKey"}`; `null` or absent is no budget. A `cancelKey`
    /// registers the call's token until the returned entry is dropped.
    fn budget(&self, j: Option<&J>) -> Res<(Option<QueryBudget>, CancelEntry<'_>)> {
        let mut entry = CancelEntry {
            db: self,
            key: None,
        };
        let Some(j) = j.filter(|j| !j.is_null()) else {
            return Ok((None, entry));
        };
        let o = j
            .as_object()
            .ok_or_else(|| arg("budget must be an object"))?;
        let mut b = QueryBudget::default();
        for (k, v) in o {
            if k == "cancelKey" {
                let key = v
                    .as_str()
                    .ok_or_else(|| arg("cancelKey must be a string"))?
                    .to_string();
                // a key cancelled before the call started keeps its cancelled token
                let token = self.cancel_map().entry(key.clone()).or_default().clone();
                b.cancel = Some(token);
                entry.key = Some(key);
                continue;
            }
            let n = v
                .as_u64()
                .ok_or_else(|| arg(format!("budget {k} must be a non-negative integer")))?;
            match k.as_str() {
                "timeoutMs" => b.timeout = Some(Duration::from_millis(n)),
                "readerTimeoutMs" => b.reader_timeout = Some(Duration::from_millis(n)),
                "maxRows" => b.max_rows = Some(n),
                "maxBytes" => b.max_bytes = Some(n),
                other => return Err(arg(format!("unknown budget option {other:?}"))),
            }
        }
        Ok((Some(b), entry))
    }

    /// Cancels the call running with `cancelKey` = `key`, or the next one to start
    /// with it. Returns whether a running call held the key.
    fn cancel(&self, key: &str) -> bool {
        let mut map = self.cancel_map();
        let running = map.contains_key(key);
        map.entry(key.to_string()).or_default().cancel();
        running
    }

    /// Runs one operation.
    ///
    /// Reads take `{"view": ..., ...}` and are `sparql` (`text`, `provenance`,
    /// `queryOnly`), `cypher` (`text`,
    /// `params`), `triples` (`s`, `p`, `o`), `path` (`start`, `path`, `mode`,
    /// `maxHops`), `events` (`since`), `graphs`, `graphMembers` (`graph`), `values`
    /// (`s`, `key`), `dependents` (`eid`: what stands on a statement) and `bundle`
    /// (`eid`: the statement with its layers and evidence, as `tiramemsu-bundle/1`
    /// JSON), `textSearch` (`text`, `mode`, `graphs`, `predicates`, `limit`,
    /// `confidence`: ranked text recall), `conflicts` (`s`, `p`, `limit`,
    /// `confidence`, `source`: distinct objects valid at the same time, with
    /// attributed evidence) and `explainSparql` (`text`: the routing
    /// of each region with its reason, the SQL and `EXPLAIN QUERY PLAN`, without
    /// running the query). `rebuildTextIndex` and `enableTextIndex`
    /// maintain the derived text index. Writes are `transact` (`ops`, `options`; the ops include `importBundle`),
    /// `cypherWrite` (`text`, `params`, `options`) and `with` (`ops`, `queries`), plus
    /// `optimize` and `info`. `previewBundle` (`bundle`, `budget`) runs a bundle
    /// import as a dry run and reports what it would do. Reads, `transact` and `cypherWrite` take an optional
    /// `budget`, and `cancel` (`key`) stops the call running with that `cancelKey`.
    /// Bulk imports are `importBegin`, `importChunk` (`session`, `ops`, `options`,
    /// `budget`), `importProgress`, `importFinish` and `importCancel` (`session`).
    /// Saved answers are `saveAnswer` (`name`, `language`, `text`, `params`,
    /// `view`, `budget`), `savedAnswer` (`name`), `savedAnswers`,
    /// `checkSavedAnswers` (the new invalidations), `refreshAnswer` (`name`,
    /// `budget`) and `deleteSavedAnswer` (`name`).
    pub fn call(&self, op: &str, args: &J) -> Res<J> {
        match op {
            "sparql" | "cypher" | "triples" | "path" | "events" | "graphs" | "graphMembers"
            | "values" | "dependents" | "bundle" | "textSearch" | "explainSparql" | "conflicts" => {
                let view = read::view_from_json(&self.db, args.get("view").unwrap_or(&J::Null))?;
                let (budget, _entry) = self.budget(args.get("budget"))?;
                match &budget {
                    // the whole call is one operation: lookups, the read and decoding
                    Some(b) => b
                        .run(|| read::run(&view, op, args).map_err(value::into_core))
                        .map_err(BindError::from),
                    None => read::run(&view, op, args),
                }
            }
            "transact" => {
                let (budget, _entry) = self.budget(args.get("budget"))?;
                tx::transact(&self.db, args, budget.as_ref())
            }
            "cypherWrite" => {
                let (budget, _entry) = self.budget(args.get("budget"))?;
                tx::cypher_write(&self.db, args, budget.as_ref())
            }
            "cancel" => {
                let key = args
                    .get("key")
                    .and_then(J::as_str)
                    .ok_or_else(|| arg("`key` must be a string"))?;
                Ok(json!({ "running": self.cancel(key) }))
            }
            "importBegin" | "importChunk" | "importProgress" | "importFinish" | "importCancel" => {
                self.import(op, args)
            }
            "saveAnswer" | "refreshAnswer" => {
                let (budget, _entry) = self.budget(args.get("budget"))?;
                saved::run(&self.db, op, args, budget.as_ref())
            }
            "savedAnswer" | "savedAnswers" | "checkSavedAnswers" | "deleteSavedAnswer" => {
                saved::run(&self.db, op, args, None)
            }
            "previewBundle" => {
                let (budget, _entry) = self.budget(args.get("budget"))?;
                match &budget {
                    Some(b) => b
                        .run(|| tx::preview_bundle(&self.db, args).map_err(value::into_core))
                        .map_err(BindError::from),
                    None => tx::preview_bundle(&self.db, args),
                }
            }
            "with" => tx::with(&self.db, args),
            "optimize" => {
                self.db.optimize()?;
                Ok(J::Null)
            }
            "rebuildTextIndex" => Ok(json!({ "values": self.db.rebuild_text_index()? })),
            "enableTextIndex" => Ok(json!({ "built": self.db.enable_text_index()? })),
            "info" => Ok(json!({
                "path": self.db.path().display().to_string(),
                "readers": self.db.reader_count(),
                "pathMaxHops": self.db.path_max_hops(),
                "importActive": self.db.import_active(),
                "statisticsDue": self.db.statistics_due()?,
            })),
            other => Err(arg(format!("unknown operation {other:?}"))),
        }
    }

    /// [`Database::call`] on JSON text, returning JSON text: the form a native class exports.
    /// A failure is the text of [`BindError::to_json`].
    pub fn call_text(&self, op: &str, args: &str) -> std::result::Result<String, String> {
        let parsed: J = if args.is_empty() {
            J::Null
        } else {
            serde_json::from_str(args).map_err(|e| {
                arg(format!("arguments are not JSON: {e}"))
                    .to_json()
                    .to_string()
            })?
        };
        self.call(op, &parsed)
            .map(|j| j.to_string())
            .map_err(|e| e.to_json().to_string())
    }
}

pub(crate) fn tx_options(j: &J) -> Res<TxOptions> {
    let mut o = TxOptions::default();
    if let Some(m) = j.as_object() {
        for (k, v) in m {
            match k.as_str() {
                "dryRun" => {
                    o.dry_run = v.as_bool().ok_or_else(|| arg("dryRun must be a boolean"))?
                }
                "maxCascade" => {
                    o.max_cascade = v
                        .as_u64()
                        .ok_or_else(|| arg("maxCascade must be an integer"))?
                        as usize
                }
                other => return Err(arg(format!("unknown transaction option {other:?}"))),
            }
        }
    } else if !j.is_null() {
        return Err(arg("options must be an object"));
    }
    Ok(o)
}
