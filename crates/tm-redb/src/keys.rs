//! Key and value encodings.
//!
//! Every integer is stored big-endian with the sign bit flipped, so that byte
//! order equals numeric order and a key prefix is a byte prefix. Absent valid-time
//! bounds and an absent retraction are sentinels that no real value takes.

/// Encodes `x` so that bytes compare like the integers.
#[inline]
pub fn enc(x: i64) -> [u8; 8] {
    ((x as u64) ^ (1 << 63)).to_be_bytes()
}

/// Decodes [`enc`].
#[inline]
pub fn dec(b: &[u8]) -> i64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[..8]);
    (u64::from_be_bytes(a) ^ (1 << 63)) as i64
}

/// The sentinel of an absent optional value.
pub const NONE: i64 = i64::MIN;

/// An optional value as an integer.
#[inline]
pub fn opt(x: Option<i64>) -> i64 {
    x.unwrap_or(NONE)
}

/// [`opt`] back to an option.
#[inline]
pub fn unopt(x: i64) -> Option<i64> {
    (x != NONE).then_some(x)
}

/// A key of up to five encoded integers.
pub fn key(parts: &[i64]) -> Vec<u8> {
    let mut k = Vec::with_capacity(parts.len() * 8);
    for p in parts {
        k.extend_from_slice(&enc(*p));
    }
    k
}

/// A key prefix of up to three encoded integers, kept on the stack so that a
/// join probe allocates nothing to seek.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefix {
    buf: [u8; 24],
    len: usize,
}

impl Prefix {
    /// The empty prefix.
    pub const EMPTY: Prefix = Prefix {
        buf: [0; 24],
        len: 0,
    };

    /// Appends an encoded integer.
    #[inline]
    pub fn push(&mut self, x: i64) {
        self.buf[self.len..self.len + 8].copy_from_slice(&enc(x));
        self.len += 8;
    }

    /// The prefix bytes.
    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// [`prefix_end`] without a heap allocation.
    pub fn end(&self) -> Option<Prefix> {
        let mut end = *self;
        while end.len > 0 {
            end.len -= 1;
            let last = end.buf[end.len];
            if last < 0xFF {
                end.buf[end.len] = last + 1;
                end.len += 1;
                return Some(end);
            }
        }
        None
    }
}

/// The `n`-th integer of a key or value.
#[inline]
pub fn at(b: &[u8], n: usize) -> i64 {
    dec(&b[n * 8..n * 8 + 8])
}

/// The exclusive upper bound of every key that starts with `prefix`; `None` for
/// the empty prefix (no bound).
pub fn prefix_end(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last < 0xFF {
            end.push(last + 1);
            return Some(end);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_is_numeric() {
        let xs = [i64::MIN, -5, -1, 0, 1, 7, i64::MAX];
        for w in xs.windows(2) {
            assert!(enc(w[0]) < enc(w[1]));
            assert_eq!(dec(&enc(w[0])), w[0]);
        }
    }

    #[test]
    fn prefix_bounds() {
        assert_eq!(prefix_end(&[1, 2]), Some(vec![1, 3]));
        assert_eq!(prefix_end(&[1, 0xFF]), Some(vec![2]));
        assert_eq!(prefix_end(&[0xFF]), None);
    }

    #[test]
    fn stack_prefix_bounds_match_prefix_end() {
        for parts in [
            vec![],
            vec![0i64],
            vec![-1],
            vec![i64::MAX],
            vec![7, -1],
            vec![3, i64::MAX, 9],
        ] {
            let mut p = Prefix::EMPTY;
            for x in &parts {
                p.push(*x);
            }
            assert_eq!(p.as_slice(), key(&parts).as_slice());
            assert_eq!(
                p.end().map(|e| e.as_slice().to_vec()),
                prefix_end(&key(&parts)),
                "{parts:?}"
            );
        }
    }
}
