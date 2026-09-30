//! Errors of the Cypher front end and their mapping to the facade error set.

use tm_core::{Dialect, Error, Span as CoreSpan};

use crate::span::Span;

/// A front-end failure. `Core` carries an error of the store (write failures).
#[derive(Debug)]
pub enum CypherError {
    /// Invalid text or a compile-time semantic error, with the offending span.
    Parse {
        /// Offending byte range of the original text.
        span: Span,
        /// What is wrong.
        msg: String,
    },
    /// A construct outside the v1 subset.
    Unsupported {
        /// The feature name.
        feature: String,
        /// Where it occurs, when known.
        span: Option<Span>,
    },
    /// A runtime expression error.
    Eval {
        /// The message.
        msg: String,
    },
    /// A store or executor error.
    Core(Error),
}

/// Result alias.
pub type CResult<T> = Result<T, CypherError>;

impl CypherError {
    /// A parse error.
    pub fn parse(span: Span, msg: impl Into<String>) -> CypherError {
        CypherError::Parse {
            span,
            msg: msg.into(),
        }
    }

    /// An unsupported-feature error.
    pub fn unsupported(feature: impl Into<String>, span: Option<Span>) -> CypherError {
        CypherError::Unsupported {
            feature: feature.into(),
            span,
        }
    }

    /// A runtime evaluation error.
    pub fn eval(msg: impl Into<String>) -> CypherError {
        CypherError::Eval { msg: msg.into() }
    }

    /// Converts to the facade error; `text` is needed to compute line and column.
    pub fn into_core(self, text: &str) -> Error {
        match self {
            CypherError::Parse { span, msg } => {
                Error::parse(Dialect::Cypher, Some(core_span(text, span.start)), msg)
            }
            CypherError::Unsupported { feature, .. } => Error::unsupported(feature),
            CypherError::Eval { msg } => Error::Eval {
                dialect: Dialect::Cypher,
                msg,
            },
            CypherError::Core(e) => e,
        }
    }
}

impl From<Error> for CypherError {
    fn from(e: Error) -> CypherError {
        CypherError::Core(e)
    }
}

/// Line, column (1-based, in characters) and byte offset of `offset`.
pub fn core_span(text: &str, offset: usize) -> CoreSpan {
    let offset = offset.min(text.len());
    let before = text.get(..offset).unwrap_or("");
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    CoreSpan {
        line,
        column: col,
        offset,
    }
}
