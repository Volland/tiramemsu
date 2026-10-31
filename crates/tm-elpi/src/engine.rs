//! The bridge to lean-elpi (cargo feature `native`): compiled programs with
//! the host builtins declared, cached by text, and solves that route each
//! builtin call to [`Host`]. Without the feature every run is `Unsupported`.

use crate::host::Host;
use crate::run::{ElpiOptions, Solved, Source};
use tm_core::Result;

#[cfg(feature = "native")]
mod native {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex, OnceLock};

    use leanelpi::{Program, Term};
    use tm_core::{Error, Result};

    use crate::host::{Host, SIGNATURES};
    use crate::run::{ElpiOptions, Sol, Solved, Source};
    use crate::term::Tree;

    const CACHED: usize = 32;

    type Cache = Mutex<Vec<(String, Arc<Program>)>>;

    fn cache() -> &'static Cache {
        static C: OnceLock<Cache> = OnceLock::new();
        C.get_or_init(|| Mutex::new(Vec::new()))
    }

    fn load_err(e: leanelpi::Error) -> Error {
        Error::ElpiProgram {
            message: e.to_string(),
        }
    }

    /// The compiled program of `text`, with the host builtins declared.
    pub fn compile(text: &str) -> Result<Arc<Program>> {
        let c = cache();
        if let Some(p) = c
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .find(|(t, _)| t == text)
            .map(|(_, p)| p.clone())
        {
            return Ok(p);
        }
        let mut p = Program::new(text).map_err(load_err)?;
        for (name, sig) in SIGNATURES {
            p = p.declare(name, sig).map_err(load_err)?;
        }
        let p = Arc::new(p);
        let mut g = c.lock().unwrap_or_else(|p| p.into_inner());
        if g.len() >= CACHED {
            g.remove(0);
        }
        g.push((text.to_string(), p.clone()));
        Ok(p)
    }

    fn to_tree(t: &Term) -> Tree {
        match t {
            Term::Const(c) => Tree::Const(c.clone()),
            Term::App(c, a) => Tree::App(c.clone(), a.iter().map(to_tree).collect()),
            Term::Int(n) => Tree::Int(*n),
            Term::Float(x) => Tree::Float(*x),
            Term::Str(s) => Tree::Str(s.clone()),
            Term::Var(v) => Tree::Var(v.clone()),
            Term::Other(o) => Tree::Other(o.clone()),
        }
    }

    fn from_tree(t: &Tree) -> Term {
        match t {
            Tree::Const(c) => Term::Const(c.clone()),
            Tree::App(c, a) => Term::App(c.clone(), a.iter().map(from_tree).collect()),
            Tree::Int(n) => Term::Int(*n),
            Tree::Float(x) => Term::Float(*x),
            Tree::Str(s) => Term::Str(s.clone()),
            Tree::Var(v) => Term::Var(v.clone()),
            Tree::Other(o) => Term::Other(o.clone()),
        }
    }

    pub fn solve<'a>(
        src: &Source,
        goal: &str,
        opts: &ElpiOptions,
        mut host: Host<'a>,
    ) -> Result<Solved<'a>> {
        let p = compile(&src.full_text())?;
        let run = p.solve_host(goal, opts.limit, opts.fuel, |name: &str, args: &[Term]| {
            let args: Vec<Tree> = args.iter().map(to_tree).collect();
            host.call(name, &args).map(|rows| {
                rows.iter()
                    .map(|r| r.iter().map(from_tree).collect())
                    .collect()
            })
        });
        let run = match run {
            Ok(r) => r,
            Err(e) if host.over_budget => {
                return Err(Error::ElpiOutOfFuel {
                    message: e.to_string(),
                })
            }
            Err(e) if e.is_out_of_fuel() => {
                return Err(Error::ElpiOutOfFuel {
                    message: e.to_string(),
                })
            }
            Err(e) => {
                return Err(Error::ElpiRun {
                    message: e.to_string(),
                })
            }
        };
        let sols = run
            .solutions
            .into_iter()
            .map(|s| Sol {
                terms: s
                    .terms
                    .iter()
                    .map(|(k, t)| (k.clone(), to_tree(t)))
                    .collect::<BTreeMap<_, _>>(),
                printed: s.bindings,
            })
            .collect();
        Ok(Solved {
            sols,
            exhausted: run.exhausted,
            fuel_used: run.fuel_used,
            host,
        })
    }
}

#[cfg(feature = "native")]
pub(crate) fn compile(text: &str) -> Result<()> {
    native::compile(text).map(|_| ())
}

#[cfg(feature = "native")]
pub(crate) fn solve<'a>(
    src: &Source,
    goal: &str,
    opts: &ElpiOptions,
    host: Host<'a>,
) -> Result<Solved<'a>> {
    native::solve(src, goal, opts, host)
}

#[cfg(not(feature = "native"))]
fn unsupported() -> tm_core::Error {
    tm_core::Error::unsupported("ELPI runs (tm-elpi built without its `native` feature)")
}

#[cfg(not(feature = "native"))]
pub(crate) fn compile(_text: &str) -> Result<()> {
    Err(unsupported())
}

#[cfg(not(feature = "native"))]
pub(crate) fn solve<'a>(
    _src: &Source,
    _goal: &str,
    _opts: &ElpiOptions,
    _host: Host<'a>,
) -> Result<Solved<'a>> {
    Err(unsupported())
}
