//! Valid-time arithmetic for conclusions and the segment sweep
//! (`lat.md/elpi#Pointwise Valid Time`).

use tm_core::Valid;

/// The intersection of intervals (`Valid::ALWAYS` for none), `None` when it
/// is empty.
pub fn intersect<'a>(all: impl IntoIterator<Item = &'a Valid>) -> Option<Valid> {
    let mut out = Valid::ALWAYS;
    for v in all {
        out.from = match (out.from, v.from) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        out.to = match (out.to, v.to) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    match (out.from, out.to) {
        (Some(f), Some(t)) if f >= t => None,
        _ => Some(out),
    }
}

/// True when `v` holds throughout the segment `seg`.
pub fn covers(v: &Valid, seg: &Valid) -> bool {
    let from_ok = match (v.from, seg.from) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => a <= b,
    };
    let to_ok = match (v.to, seg.to) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(a), Some(b)) => a >= b,
    };
    from_ok && to_ok
}

/// The segments the bounds of `intervals` cut the timeline into, in order.
pub fn segments<'a>(intervals: impl IntoIterator<Item = &'a Valid>) -> Vec<Valid> {
    let mut cuts: Vec<i64> = intervals
        .into_iter()
        .flat_map(|v| [v.from, v.to])
        .flatten()
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out = Vec::with_capacity(cuts.len() + 1);
    let mut from = None;
    for c in cuts {
        out.push(Valid { from, to: Some(c) });
        from = Some(c);
    }
    out.push(Valid { from, to: None });
    out
}

/// Merges ordered segments into maximal intervals (neighbours that touch).
pub fn merge(mut segs: Vec<Valid>) -> Vec<Valid> {
    segs.sort_by_key(|v| (v.from.is_some(), v.from));
    let mut out: Vec<Valid> = Vec::new();
    for s in segs {
        if let Some(last) = out.last_mut() {
            if last.to.is_some() && last.to == s.from {
                last.to = s.to;
                continue;
            }
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(from: Option<i64>, to: Option<i64>) -> Valid {
        Valid { from, to }
    }

    #[test]
    fn intersections() {
        assert_eq!(
            intersect(&[v(Some(2020), Some(2025)), v(Some(2022), None)]),
            Some(v(Some(2022), Some(2025)))
        );
        assert_eq!(intersect(&[v(Some(1), Some(2)), v(Some(2), Some(3))]), None);
        assert_eq!(intersect(&[]), Some(Valid::ALWAYS));
    }

    #[test]
    fn sweep_segments() {
        let s = segments(&[v(Some(1), Some(3)), v(Some(2), None)]);
        assert_eq!(s.len(), 4);
        assert!(covers(&v(Some(1), Some(3)), &s[1]));
        assert!(!covers(&v(Some(1), Some(3)), &s[3]));
        assert_eq!(merge(vec![s[1], s[2]]), vec![v(Some(1), Some(3))]);
    }
}
