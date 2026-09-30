//! Path rows and the `path_json` text of the `tm_path` table function.

use tm_core::{ObjectId, Tag};
use tm_ir::vocab::{SYS_OBJECT, SYS_PREDICATE, SYS_SUBJECT};

pub use super::automaton::Dir;

/// What a hop traversed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HopKind {
    /// A stored statement.
    Stored = 0,
    /// The virtual `sys:subject` hop.
    Subject = 1,
    /// The virtual `sys:object` hop.
    Object = 2,
    /// The virtual `sys:predicate` hop.
    Predicate = 3,
}

impl HopKind {
    /// The reserved id naming a virtual predicate in path values (see
    /// [`virtual_pred_id`]); `None` for a stored hop.
    pub fn virtual_pred(self) -> Option<ObjectId> {
        (self != HopKind::Stored).then(|| virtual_pred_id(self))
    }
}

/// Payload base of the reserved ids of the virtual predicates. Real dictionary ids
/// count up from 1 and never get here (plan-local ids start at `1 << 58`), so the ids need no dictionary entry (a
/// reader cannot insert one) and never collide with a stored predicate.
const VIRTUAL_BASE: u64 = 1 << 59;

/// The id that names virtual hop `kind` in a path value (`p` of its edge entry).
/// It is an `IRI`-tagged id outside the dictionary: decode it with
/// [`virtual_pred_iri`], not through the term dictionary.
pub fn virtual_pred_id(kind: HopKind) -> ObjectId {
    ObjectId::from_unsigned(Tag::Iri, VIRTUAL_BASE + kind as u64)
}

/// The IRI a reserved virtual-predicate id stands for.
pub fn virtual_pred_iri(id: ObjectId) -> Option<&'static str> {
    [
        (HopKind::Subject, SYS_SUBJECT),
        (HopKind::Object, SYS_OBJECT),
        (HopKind::Predicate, SYS_PREDICATE),
    ]
    .into_iter()
    .find(|(k, _)| virtual_pred_id(*k) == id)
    .map(|(_, iri)| iri)
}

/// One traversed hop.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hop {
    /// The statement traversed.
    pub eid: ObjectId,
    /// Its predicate (the reserved virtual id for a virtual hop).
    pub pred: ObjectId,
    /// Forward (`Out`, subject to object) or inverse (`In`).
    pub dir: Dir,
    /// Stored or virtual.
    pub kind: HopKind,
}

impl Hop {
    /// The identity that may not repeat on a trail: a stored hop is its eid
    /// (whatever the direction), a virtual hop is `(eid, kind)`.
    pub fn identity(&self) -> (i64, u8) {
        (self.eid.raw(), self.kind as u8)
    }
}

/// A path value: `hops + 1` nodes and `hops` hop entries, in traversal order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path {
    /// The nodes, start first.
    pub nodes: Vec<ObjectId>,
    /// The hops.
    pub hops: Vec<Hop>,
}

impl Path {
    /// Parses the [`Path::to_json`] text (`None` when it is not in that format).
    pub fn from_json(text: &str) -> Option<Path> {
        use std::sync::OnceLock;
        static EDGE: OnceLock<regex::Regex> = OnceLock::new();
        let edge = EDGE.get_or_init(|| {
            regex::Regex::new(r#"\{"eid":(-?\d+),"p":(-?\d+),"dir":"(out|in)"\}"#).expect("regex")
        });
        let rest = text.strip_prefix("{\"nodes\":[")?;
        let (nodes, rest) = rest.split_once("],\"edges\":[")?;
        let nodes = nodes
            .split(',')
            .filter(|n| !n.is_empty())
            .map(|n| n.parse::<i64>().ok().map(ObjectId::from_raw))
            .collect::<Option<Vec<_>>>()?;
        let hops = edge
            .captures_iter(rest)
            .map(|c| {
                let eid = ObjectId::from_raw(c[1].parse().ok()?);
                let pred = ObjectId::from_raw(c[2].parse().ok()?);
                let kind = match virtual_pred_iri(pred) {
                    Some(SYS_SUBJECT) => HopKind::Subject,
                    Some(SYS_OBJECT) => HopKind::Object,
                    Some(_) => HopKind::Predicate,
                    None => HopKind::Stored,
                };
                Some(Hop {
                    eid,
                    pred,
                    dir: if &c[3] == "out" { Dir::Out } else { Dir::In },
                    kind,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Path { nodes, hops })
    }

    /// The path read backwards: nodes and hops reversed, every direction flipped.
    pub fn reversed(mut self) -> Path {
        self.nodes.reverse();
        self.hops.reverse();
        for h in &mut self.hops {
            h.dir = match h.dir {
                Dir::Out => Dir::In,
                Dir::In => Dir::Out,
            };
        }
        self
    }

    /// The `path_json` text: `{"nodes":[…],"edges":[{"eid":…,"p":…,"dir":"out"|"in"},…]}`.
    pub fn to_json(&self) -> String {
        let mut s = String::from("{\"nodes\":[");
        for (i, n) in self.nodes.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&n.raw().to_string());
        }
        s.push_str("],\"edges\":[");
        for (i, h) in self.hops.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            let dir = match h.dir {
                Dir::Out => "out",
                Dir::In => "in",
            };
            s.push_str(&format!(
                "{{\"eid\":{},\"p\":{},\"dir\":\"{dir}\"}}",
                h.eid.raw(),
                h.pred.raw()
            ));
        }
        s.push_str("]}");
        s
    }
}

/// One result row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathRow {
    /// The start.
    pub start: ObjectId,
    /// The end.
    pub end: ObjectId,
    /// The hop count (the shortest witness length in `REACH` mode).
    pub hops: u32,
    /// The path value; `None` in `REACH` mode.
    pub path: Option<Path>,
    /// The arrival of a time-respecting search, in epoch ms: the row's final time
    /// (in `REACH` mode the earliest over every time-respecting walk to the end).
    /// `None` when the search is not time-respecting, and when the arrival is −∞
    /// (no start instant and no traversed statement with a valid-time start).
    pub arrival: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> ObjectId {
        ObjectId::from_unsigned(Tag::Iri, n)
    }

    // path-table-function "path_json for a trail" / "Zero-length row"
    #[test]
    fn json_shape() {
        let zero = Path {
            nodes: vec![id(1)],
            hops: vec![],
        };
        assert_eq!(
            zero.to_json(),
            format!("{{\"nodes\":[{}],\"edges\":[]}}", id(1).raw())
        );
        let p = Path {
            nodes: vec![id(1), id(2)],
            hops: vec![Hop {
                eid: ObjectId::from_unsigned(Tag::Stmt, 7),
                pred: id(3),
                dir: Dir::In,
                kind: HopKind::Stored,
            }],
        };
        let j = p.to_json();
        assert!(j.contains("\"dir\":\"in\""), "{j}");
        assert!(j.contains(&format!(
            "\"eid\":{}",
            ObjectId::from_unsigned(Tag::Stmt, 7).raw()
        )));
    }

    #[test]
    fn json_round_trip_and_reversal() {
        let p = Path {
            nodes: vec![id(1), id(2), id(3)],
            hops: vec![
                Hop {
                    eid: ObjectId::from_unsigned(Tag::Stmt, 7),
                    pred: id(4),
                    dir: Dir::Out,
                    kind: HopKind::Stored,
                },
                Hop {
                    eid: ObjectId::from_unsigned(Tag::Stmt, 8),
                    pred: virtual_pred_id(HopKind::Object),
                    dir: Dir::In,
                    kind: HopKind::Object,
                },
            ],
        };
        assert_eq!(Path::from_json(&p.to_json()), Some(p.clone()));
        let r = p.clone().reversed();
        assert_eq!(r.nodes, vec![id(3), id(2), id(1)]);
        assert_eq!((r.hops[0].dir, r.hops[1].dir), (Dir::Out, Dir::In));
        assert_eq!(r.hops[0].eid, p.hops[1].eid);
        assert_eq!(r.reversed(), p);
        assert!(Path::from_json("[]").is_none());
    }

    #[test]
    fn virtual_ids_round_trip() {
        for (k, iri) in [
            (HopKind::Subject, SYS_SUBJECT),
            (HopKind::Object, SYS_OBJECT),
            (HopKind::Predicate, SYS_PREDICATE),
        ] {
            assert_eq!(virtual_pred_iri(virtual_pred_id(k)), Some(iri));
        }
        assert_eq!(virtual_pred_iri(id(5)), None);
        assert_eq!(HopKind::Stored.virtual_pred(), None);
    }
}
