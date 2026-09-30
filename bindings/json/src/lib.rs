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
// @lat: [[api#Bindings]]

mod read;
mod tx;
pub mod value;

use serde_json::{json, Value as J};
use std::time::Duration;
use tiramemsu::{Db, Error, OpenOptions, TxOptions};

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
                Error::Reentrant => "Reentrant",
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
    db: Db,
}

impl Database {
    /// Opens (creating if needed) the database file at `path`. `options` may carry
    /// `readers`, `busyTimeoutMs`, `termCacheCapacity`, `optimizeEvery`,
    /// `pathMaxHops` and `pathMaxStates`; anything else is rejected.
    pub fn open(path: &str, options: &J) -> Res<Database> {
        let mut opts = OpenOptions::default();
        if let Some(o) = options.as_object() {
            for (k, v) in o {
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
                    other => return Err(arg(format!("unknown option {other:?}"))),
                }
            }
        } else if !options.is_null() {
            return Err(arg("options must be an object"));
        }
        Ok(Database {
            db: Db::open(path, opts)?,
        })
    }

    /// Runs one operation.
    ///
    /// Reads take `{"view": ..., ...}` and are `sparql` (`text`), `cypher` (`text`,
    /// `params`), `triples` (`s`, `p`, `o`), `path` (`start`, `path`, `mode`,
    /// `maxHops`), `events` (`since`), `graphs`, `graphMembers` (`graph`), `values`
    /// (`s`, `key`), `dependents` (`eid`: what stands on a statement) and `bundle`
    /// (`eid`: the statement with its layers and evidence, as `tiramemsu-bundle/1`
    /// JSON). Writes are `transact` (`ops`, `options`; the ops include `importBundle`),
    /// `cypherWrite` (`text`, `params`, `options`) and `with` (`ops`, `queries`), plus
    /// `optimize` and `info`.
    pub fn call(&self, op: &str, args: &J) -> Res<J> {
        match op {
            "sparql" | "cypher" | "triples" | "path" | "events" | "graphs" | "graphMembers"
            | "values" | "dependents" | "bundle" => {
                let view = read::view_from_json(&self.db, args.get("view").unwrap_or(&J::Null))?;
                read::run(&view, op, args)
            }
            "transact" => tx::transact(&self.db, args),
            "cypherWrite" => tx::cypher_write(&self.db, args),
            "with" => tx::with(&self.db, args),
            "optimize" => {
                self.db.optimize()?;
                Ok(J::Null)
            }
            "info" => Ok(json!({
                "path": self.db.path().display().to_string(),
                "readers": self.db.reader_count(),
                "pathMaxHops": self.db.path_max_hops(),
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
