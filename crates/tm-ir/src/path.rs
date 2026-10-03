//! Path expressions and modes (the interface; the algorithm is `add-path-engine`).

use tm_core::Value;

use crate::term::TermOrVar;
use crate::var::Var;

/// A property-path expression.
///
/// # Example
///
/// ```
/// use tm_ir::PathExpr;
///
/// // knows+ / ^manages
/// let p = PathExpr::Seq(vec![
///     PathExpr::iri("urn:tiramemsu:v:knows").plus(),
///     PathExpr::iri("urn:tiramemsu:v:manages").inverse(),
/// ]);
/// assert!(matches!(p, PathExpr::Seq(ref v) if v.len() == 2));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum PathExpr {
    /// One predicate hop (an IRI; `sys:anyRelationship` is the Cypher wildcard).
    Pred(Value),
    /// `^p`: the hop backwards.
    Inverse(Box<PathExpr>),
    /// `a / b / …`.
    Seq(Vec<PathExpr>),
    /// `a | b | …`.
    Alt(Vec<PathExpr>),
    /// `p*`.
    ZeroOrMore(Box<PathExpr>),
    /// `p+`.
    OneOrMore(Box<PathExpr>),
    /// `p?`.
    ZeroOrOne(Box<PathExpr>),
    /// `p{min,max}`; `max = None` is unbounded.
    Repeat {
        /// The repeated path.
        inner: Box<PathExpr>,
        /// Minimum count.
        min: u32,
        /// Maximum count, `None` for unbounded.
        max: Option<u32>,
    },
}

impl PathExpr {
    /// A predicate hop on an IRI.
    pub fn iri(iri: impl Into<String>) -> PathExpr {
        PathExpr::Pred(Value::Iri(iri.into()))
    }

    /// `^self`.
    pub fn inverse(self) -> PathExpr {
        match self {
            PathExpr::Inverse(p) => *p,
            p => PathExpr::Inverse(Box::new(p)),
        }
    }

    /// `self*`.
    pub fn star(self) -> PathExpr {
        PathExpr::ZeroOrMore(Box::new(self))
    }

    /// `self+`.
    pub fn plus(self) -> PathExpr {
        PathExpr::OneOrMore(Box::new(self))
    }
}

/// How the path operator enumerates paths.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum PathMode {
    /// Endpoints only, set semantics (SPARQL).
    #[default]
    Reachability,
    /// No repeated relationship eid (Cypher).
    Trail,
    /// One shortest path per endpoint pair.
    AnyShortest,
    /// Every shortest path per endpoint pair.
    AllShortest,
}

impl PathMode {
    /// Short spelling of [`PathMode::Reachability`].
    #[allow(non_upper_case_globals)]
    pub const Reach: PathMode = PathMode::Reachability;

    /// The `mode` argument text of `tm_path`.
    pub fn sql_name(self) -> &'static str {
        match self {
            PathMode::Reachability => "REACH",
            PathMode::Trail => "TRAIL",
            PathMode::AnyShortest => "ANY_SHORTEST",
            PathMode::AllShortest => "ALL_SHORTEST",
        }
    }
}

impl std::fmt::Display for PathMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.sql_name())
    }
}

impl std::str::FromStr for PathMode {
    type Err = String;

    /// Parses `REACH | TRAIL | ANY_SHORTEST | ALL_SHORTEST` (case-insensitive);
    /// any other mode (`WALK`, `SIMPLE`, `ACYCLIC`, `SHORTEST`) is rejected.
    fn from_str(s: &str) -> Result<PathMode, String> {
        match s.trim().to_ascii_uppercase().as_str() {
            "REACH" => Ok(PathMode::Reachability),
            "TRAIL" => Ok(PathMode::Trail),
            "ANY_SHORTEST" => Ok(PathMode::AnyShortest),
            "ALL_SHORTEST" => Ok(PathMode::AllShortest),
            _ => Err(format!("unsupported path mode `{s}`")),
        }
    }
}

/// The time-respecting (causal) modifier of a path pattern: valid time never goes
/// backwards along the path, a journey in a temporal graph
/// (`lat.md/query#Temporal Path Syntax`).
///
/// A search time τ starts at `after` (−∞ when `None`); a stored hop over a
/// statement valid `[v_from, v_to)` needs `v_to` unbounded or `v_to > τ` and moves
/// τ to `max(τ, v_from)`; virtual hops keep τ. The SPARQL scope `SERVICE
/// <urn:tiramemsu:tm:timeRespecting…>` and the Cypher `MATCH TIME RESPECTING`
/// lower to it; the executor runs it on the native path operator.
///
/// ```
/// use tm_core::Value;
/// use tm_ir::{TemporalPath, TermOrVar, Var};
///
/// // from 2024-06-01T00:00:00Z, binding the earliest arrival to ?t
/// let t = TemporalPath {
///     after: Some(TermOrVar::Const(Value::Int(1_717_200_000_000))),
///     arrival: Some(Var::new("t")),
/// };
/// assert!(t.arrival.is_some());
/// assert_eq!(TemporalPath::default().after, None); // from −∞
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TemporalPath {
    /// The start instant: `None` is −∞; otherwise a constant integer of epoch
    /// milliseconds, an `xsd:date` / `xsd:dateTime` constant, or a parameter
    /// holding one of these.
    pub after: Option<TermOrVar>,
    /// Binds the arrival of each row as an integer of epoch milliseconds (in
    /// `REACH` the earliest over every journey to the end); unbound when the arrival
    /// is −∞ (no `after` and no traversed statement with a `v_from`).
    pub arrival: Option<Var>,
}

/// How completely a path search was evaluated: it ran out of states to expand,
/// or a hop limit stopped it with states left (`lat.md/query#Physical
/// Planning#Path Engine#Path Completeness`).
///
/// State exhaustion is never reported here: a search over its state guard fails
/// with `PathLimitExceeded` instead of returning a prefix.
///
/// ```
/// use tm_ir::PathCompleteness;
///
/// let a = PathCompleteness::Exhaustive;
/// let b = PathCompleteness::StoppedAtBound { max_hops: 3 };
/// let c = PathCompleteness::StoppedAtCap { max_hops: 15 };
/// assert!(a.is_complete() && b.is_complete() && !c.is_complete());
/// assert_eq!(a.merge(b), b); // several searches report the least complete one
/// assert_eq!(b.merge(c), c);
/// assert_eq!(c.kind(), "cap");
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PathCompleteness {
    /// No state was left to expand: every match was found, whatever the bound.
    Exhaustive,
    /// The explicit hop bound of the query (`{m,n}`, Cypher `*..n`, a `max_hops`
    /// argument) stopped the search with states left: the result is complete
    /// within that bound, as asked.
    StoppedAtBound {
        /// The bound.
        max_hops: u32,
    },
    /// The configured hop cap (`OpenOptions::path_max_hops`) stopped an unbounded
    /// expression with states left: longer matches may exist, so the result is
    /// not an exhaustive evaluation of the expression.
    StoppedAtCap {
        /// The cap.
        max_hops: u32,
    },
}

impl PathCompleteness {
    /// True unless the configured hop cap cut the search.
    pub fn is_complete(self) -> bool {
        !matches!(self, PathCompleteness::StoppedAtCap { .. })
    }

    /// The short name: `exhaustive`, `bound` or `cap`.
    pub fn kind(self) -> &'static str {
        match self {
            PathCompleteness::Exhaustive => "exhaustive",
            PathCompleteness::StoppedAtBound { .. } => "bound",
            PathCompleteness::StoppedAtCap { .. } => "cap",
        }
    }

    /// The hop limit that stopped the search, if one did.
    pub fn max_hops(self) -> Option<u32> {
        match self {
            PathCompleteness::Exhaustive => None,
            PathCompleteness::StoppedAtBound { max_hops }
            | PathCompleteness::StoppedAtCap { max_hops } => Some(max_hops),
        }
    }

    fn rank(self) -> u8 {
        match self {
            PathCompleteness::Exhaustive => 0,
            PathCompleteness::StoppedAtBound { .. } => 1,
            PathCompleteness::StoppedAtCap { .. } => 2,
        }
    }

    /// The verdict over two searches: the less complete one; of two stopped by the
    /// same kind of limit, the one with the smaller limit.
    pub fn merge(self, other: PathCompleteness) -> PathCompleteness {
        match self.rank().cmp(&other.rank()) {
            std::cmp::Ordering::Less => other,
            std::cmp::Ordering::Greater => self,
            std::cmp::Ordering::Equal => match (self.max_hops(), other.max_hops()) {
                (Some(a), Some(b)) if b < a => other,
                _ => self,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_names_round_trip() {
        for m in [
            PathMode::Reachability,
            PathMode::Trail,
            PathMode::AnyShortest,
            PathMode::AllShortest,
        ] {
            assert_eq!(m.to_string().parse::<PathMode>().unwrap(), m);
            assert_eq!(m.to_string().to_lowercase().parse::<PathMode>().unwrap(), m);
        }
        for bad in ["WALK", "SIMPLE", "ACYCLIC", "SHORTEST", ""] {
            assert!(bad.parse::<PathMode>().is_err(), "{bad}");
        }
    }
}
