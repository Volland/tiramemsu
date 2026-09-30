//! Byte spans into the original query text.

/// A half-open byte range of the original query text (extensions included).
///
/// `USE` clauses and `REPEATABLE ELEMENTS` are blanked before parsing without
/// moving any byte, so a span always indexes the text the caller wrote.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

impl Span {
    /// A span of `start..end`.
    pub const fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    /// The smallest span covering both.
    pub fn cover(self, o: Span) -> Span {
        Span::new(self.start.min(o.start), self.end.max(o.end))
    }

    /// The text of the span, if it is in range.
    pub fn text(self, src: &str) -> Option<&str> {
        src.get(self.start..self.end)
    }
}

impl From<open_cypher::Span> for Span {
    fn from(s: open_cypher::Span) -> Span {
        Span::new(s.start, s.end)
    }
}
