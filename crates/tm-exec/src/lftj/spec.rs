//! The plan of one native cyclic region, as the text argument of `tm_lftj`.
//!
//! The planner numbers the region's variables in join order and writes one
//! segment per triple pattern, so the operator needs no state beyond its
//! argument:
//!
//! ```text
//! lftj1;<vars>;<out var>,<out var>,…|<s> <p> <o> <eid> <tx> <valid> <c|b> <group|->|…
//! ```
//!
//! A position is `?N` (variable `N`) or `#N` (raw ObjectId). `tx` is `now`,
//! `asof:<t>` or `history`; `valid` is `-` or epoch ms. `c` adds the canonical-eid
//! predicate (set semantics), `b` keeps every eid (bag semantics). The group is the
//! Cypher isomorphism group of the pattern.

use tm_core::{ObjectId, ValidSel};

use crate::scan::{ResolvedTx, ResolvedView};

/// A pattern position.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Variable `N` of the region (its join-order rank).
    Var(usize),
    /// A constant.
    Id(ObjectId),
}

/// One triple pattern of a native region.
#[derive(Clone, Debug, PartialEq)]
pub struct LftjPattern {
    /// Subject.
    pub s: Slot,
    /// Predicate.
    pub p: Slot,
    /// Object.
    pub o: Slot,
    /// The eid variable (a hidden one when the query binds none, so parallel
    /// statements keep their multiplicity).
    pub eid: usize,
    /// The pattern's own view.
    pub view: ResolvedView,
    /// The canonical-eid predicate (one row per `(s, p, o)` under set semantics).
    pub canonical: bool,
    /// The relationship-isomorphism group (Cypher).
    pub iso_group: Option<u32>,
}

/// The plan of one native region.
#[derive(Clone, Debug, PartialEq)]
pub struct LftjSpec {
    /// Number of variables; variable `N` is bound `N`-th.
    pub vars: usize,
    /// The variables of the output columns `c0, c1, …`.
    pub out: Vec<usize>,
    /// The patterns.
    pub patterns: Vec<LftjPattern>,
}

const MAGIC: &str = "lftj1";

fn slot_text(s: &Slot) -> String {
    match s {
        Slot::Var(v) => format!("?{v}"),
        Slot::Id(id) => format!("#{}", id.raw()),
    }
}

fn parse_slot(t: &str) -> Result<Slot, String> {
    if let Some(v) = t.strip_prefix('?') {
        v.parse()
            .map(Slot::Var)
            .map_err(|_| format!("bad variable {t:?}"))
    } else if let Some(n) = t.strip_prefix('#') {
        n.parse()
            .map(|n| Slot::Id(ObjectId::from_raw(n)))
            .map_err(|_| format!("bad id {t:?}"))
    } else {
        Err(format!("bad position {t:?}"))
    }
}

impl LftjSpec {
    /// The argument text.
    pub fn to_text(&self) -> String {
        let out: Vec<String> = self.out.iter().map(usize::to_string).collect();
        let mut s = format!("{MAGIC};{};{}", self.vars, out.join(","));
        for p in &self.patterns {
            let tx = match p.view.tx {
                ResolvedTx::Now => "now".to_string(),
                ResolvedTx::AsOf(t) => format!("asof:{t}"),
                ResolvedTx::History => "history".to_string(),
            };
            let valid = match p.view.valid {
                ValidSel::Unfiltered => "-".to_string(),
                ValidSel::At(d) => d.to_string(),
            };
            s.push_str(&format!(
                "|{} {} {} ?{} {tx} {valid} {} {}",
                slot_text(&p.s),
                slot_text(&p.p),
                slot_text(&p.o),
                p.eid,
                if p.canonical { "c" } else { "b" },
                p.iso_group.map_or("-".to_string(), |g| g.to_string()),
            ));
        }
        s
    }

    /// Parses argument text, checking that every variable is in range and bound
    /// by some pattern.
    pub fn parse(text: &str) -> Result<LftjSpec, String> {
        let mut segs = text.split('|');
        let head = segs.next().unwrap_or_default();
        let mut h = head.split(';');
        if h.next() != Some(MAGIC) {
            return Err(format!("expected {MAGIC} plan text"));
        }
        let vars: usize = h
            .next()
            .and_then(|n| n.parse().ok())
            .ok_or("bad variable count")?;
        let out = match h.next() {
            Some("") | None => Vec::new(),
            Some(list) => list
                .split(',')
                .map(|x| x.parse::<usize>().map_err(|_| format!("bad output {x:?}")))
                .collect::<Result<Vec<_>, _>>()?,
        };
        let mut patterns = Vec::new();
        for seg in segs {
            let f: Vec<&str> = seg.split(' ').collect();
            let [s, p, o, e, tx, valid, sem, group] = f.as_slice() else {
                return Err(format!("bad pattern {seg:?}"));
            };
            let eid = match parse_slot(e)? {
                Slot::Var(v) => v,
                Slot::Id(_) => return Err(format!("bad eid {e:?}")),
            };
            let tx = match *tx {
                "now" => ResolvedTx::Now,
                "history" => ResolvedTx::History,
                t => ResolvedTx::AsOf(
                    t.strip_prefix("asof:")
                        .and_then(|n| n.parse().ok())
                        .ok_or_else(|| format!("bad transaction view {t:?}"))?,
                ),
            };
            let valid = match *valid {
                "-" => ValidSel::Unfiltered,
                d => ValidSel::At(d.parse().map_err(|_| format!("bad valid time {d:?}"))?),
            };
            let canonical = match *sem {
                "c" => true,
                "b" => false,
                x => return Err(format!("bad semantics {x:?}")),
            };
            let iso_group = match *group {
                "-" => None,
                g => Some(g.parse().map_err(|_| format!("bad group {g:?}"))?),
            };
            patterns.push(LftjPattern {
                s: parse_slot(s)?,
                p: parse_slot(p)?,
                o: parse_slot(o)?,
                eid,
                view: ResolvedView { tx, valid },
                canonical,
                iso_group,
            });
        }
        let spec = LftjSpec {
            vars,
            out,
            patterns,
        };
        let mut seen = vec![false; vars];
        for p in &spec.patterns {
            for v in p.var_list() {
                *seen
                    .get_mut(v)
                    .ok_or(format!("variable {v} out of range"))? = true;
            }
        }
        if seen.iter().any(|b| !b) || spec.out.iter().any(|v| *v >= vars) {
            return Err("a variable is bound by no pattern".to_string());
        }
        if spec.patterns.is_empty() {
            return Err("no patterns".to_string());
        }
        Ok(spec)
    }
}

impl LftjPattern {
    /// The distinct variables of the pattern, ascending (its trie order).
    pub fn var_list(&self) -> Vec<usize> {
        let mut vs: Vec<usize> = [self.s, self.p, self.o]
            .iter()
            .filter_map(|s| match s {
                Slot::Var(v) => Some(*v),
                Slot::Id(_) => None,
            })
            .chain(std::iter::once(self.eid))
            .collect();
        vs.sort_unstable();
        vs.dedup();
        vs
    }

    /// The constant predicate, if any.
    pub fn pred(&self) -> Option<ObjectId> {
        match self.p {
            Slot::Id(id) => Some(id),
            Slot::Var(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let spec = LftjSpec {
            vars: 5,
            out: vec![0, 1, 2],
            patterns: vec![
                LftjPattern {
                    s: Slot::Var(0),
                    p: Slot::Id(ObjectId::from_raw(-77)),
                    o: Slot::Var(1),
                    eid: 3,
                    view: ResolvedView::NOW,
                    canonical: true,
                    iso_group: None,
                },
                LftjPattern {
                    s: Slot::Var(1),
                    p: Slot::Var(2),
                    o: Slot::Var(0),
                    eid: 4,
                    view: ResolvedView {
                        tx: ResolvedTx::AsOf(12),
                        valid: ValidSel::At(-5),
                    },
                    canonical: false,
                    iso_group: Some(2),
                },
            ],
        };
        let t = spec.to_text();
        assert_eq!(LftjSpec::parse(&t).unwrap(), spec);
        assert!(LftjSpec::parse("nope").is_err());
        assert!(LftjSpec::parse("lftj1;3;0|?0 #1 ?1 ?2 now - c -").is_ok());
        assert!(LftjSpec::parse("lftj1;4;0|?0 #1 ?1 ?2 now - c -").is_err());
        assert!(LftjSpec::parse("lftj1;2;0|?0 #1 ?1 ?9 now - c -").is_err());
        assert!(LftjSpec::parse("lftj1;1;0").is_err());
    }
}
