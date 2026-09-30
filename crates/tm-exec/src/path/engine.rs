//! The path engine behind all three surfaces: it parses the text, compiles (and
//! caches) the DFA, resolves atoms and the view on the calling connection, and runs
//! the search of the mode.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use lru::LruCache;
use tm_core::{Executor, ObjectId, Result, ViewSpec, Vocab};
use tm_ir::display::path_text_canonical;
use tm_ir::PathMode;

use super::automaton::Dfa;
use super::fetch::{Fetcher, DEFAULT_BATCH};
use super::resolve::resolve;
use super::row::PathRow;
use super::search::{search, Ctx, Sink, StateBudget};
use super::syntax::{self, VocabSource};
use super::view::resolve_view;

/// Per-database path settings (`OpenOptions::path_max_hops`, `path_max_states`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PathOptions {
    /// The Cypher cap on unbounded patterns, and the `TRAIL` default of `tm_path`.
    pub max_hops: u32,
    /// The search-state guard.
    pub max_states: usize,
    /// Frontier nodes per `rarray` chunk.
    pub batch: usize,
}

impl Default for PathOptions {
    fn default() -> Self {
        PathOptions {
            max_hops: 15,
            max_states: 1_000_000,
            batch: DEFAULT_BATCH,
        }
    }
}

/// One evaluation request.
#[derive(Clone, Debug)]
pub struct PathRequest<'a> {
    /// The start node.
    pub start: ObjectId,
    /// The path text.
    pub path: &'a str,
    /// The mode.
    pub mode: PathMode,
    /// The hop bound; `None` = unbounded.
    pub max_hops: Option<u32>,
    /// The view of every hop.
    pub view: ViewSpec,
    /// Only rows for this end.
    pub end: Option<ObjectId>,
}

/// Reads the vocabulary from the database the first time a name needs it.
pub(super) struct Lazy<'e> {
    exec: &'e mut dyn Executor,
    vocab: Option<Vocab>,
}

impl<'e> Lazy<'e> {
    pub(super) fn new(exec: &'e mut dyn Executor) -> Lazy<'e> {
        Lazy { exec, vocab: None }
    }
}

impl VocabSource for Lazy<'_> {
    fn vocab(&mut self) -> Result<&Vocab> {
        if self.vocab.is_none() {
            self.vocab = Some(Vocab::load(self.exec)?);
        }
        Ok(self.vocab.as_ref().expect("loaded"))
    }
}

/// The path engine of one database.
// @lat: [[query#Physical Planning#Path Engine]]
pub struct PathEngine {
    opts: PathOptions,
    dfas: Mutex<LruCache<String, Arc<Dfa>>>,
}

impl std::fmt::Debug for PathEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PathEngine")
            .field("opts", &self.opts)
            .finish()
    }
}

impl PathEngine {
    /// An engine with these settings.
    pub fn new(opts: PathOptions) -> PathEngine {
        PathEngine {
            opts,
            dfas: Mutex::new(LruCache::new(NonZeroUsize::new(64).expect("non-zero"))),
        }
    }

    /// The settings.
    pub fn options(&self) -> &PathOptions {
        &self.opts
    }

    /// The compiled automaton of canonical path text (cached).
    fn dfa(&self, expr: &tm_ir::PathExpr) -> Result<Arc<Dfa>> {
        let key = path_text_canonical(expr);
        if let Some(d) = self
            .dfas
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
        {
            return Ok(d.clone());
        }
        let d = Arc::new(Dfa::compile(expr)?);
        self.dfas
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .put(key, d.clone());
        Ok(d)
    }

    /// Runs a request, feeding rows to `sink` in the deterministic order of the
    /// mode; the sink returns `false` to stop early.
    pub fn run(
        &self,
        exec: &mut dyn Executor,
        req: &PathRequest<'_>,
        sink: Sink<'_>,
    ) -> Result<()> {
        let expr = {
            let mut lazy = Lazy::new(exec);
            syntax::parse(req.path, &mut lazy)?
        };
        let dfa = self.dfa(&expr)?;
        let Some(view) = resolve_view(exec, req.view)? else {
            return Ok(());
        };
        let res = resolve(exec, &dfa)?;
        let mut fetch = Fetcher::new(exec, view).with_batch(self.opts.batch);
        let mut budget = StateBudget::new(self.opts.max_states);
        let mut ctx = Ctx {
            dfa: &dfa,
            res: &res,
            fetch: &mut fetch,
            budget: &mut budget,
            max_hops: req.max_hops,
            end_filter: req.end,
            start: req.start,
        };
        search(req.mode, &mut ctx, sink)
    }

    /// Runs a request and collects every row.
    pub fn eval(&self, exec: &mut dyn Executor, req: &PathRequest<'_>) -> Result<Vec<PathRow>> {
        let mut rows = Vec::new();
        self.run(exec, req, &mut |r| {
            rows.push(r);
            Ok(true)
        })?;
        Ok(rows)
    }
}
