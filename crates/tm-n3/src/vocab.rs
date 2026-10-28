//! The IRIs of N3 reasoning: the stored `n3:` vocabulary, the virtual `tm:`
//! predicates rules may read, and the builtins the monotonicity check names.

/// `n3:` namespace: rule sets and the labels of materialized conclusions.
pub const N3: &str = "urn:tiramemsu:n3:";
/// `n3:source`: `(rule_set n3:source "<N3 text>")`, the one live statement of a
/// stored rule set.
pub const N3_SOURCE: &str = "urn:tiramemsu:n3:source";
/// `n3:derivedBy`: `(D n3:derivedBy rule_set)`, the rule set that concluded a
/// materialized statement (a label, not support).
pub const N3_DERIVED_BY: &str = "urn:tiramemsu:n3:derivedBy";
/// `n3:rule`: `(D n3:rule i)`, the index of the rule that concluded it.
pub const N3_RULE: &str = "urn:tiramemsu:n3:rule";
/// `n3:skolem`: the unique key of a node invented for a blank node of a conclusion.
pub const N3_SKOLEM: &str = "urn:tiramemsu:n3:skolem";

/// `tm:` namespace (reserved: virtual predicates, never stored).
pub const TM: &str = "urn:tiramemsu:tm:";
/// `e tm:content { s p o }`: the content of loaded statement `e`.
pub const TM_CONTENT: &str = "urn:tiramemsu:tm:content";
/// `e tm:validFrom dt`: the bounded start of `e`'s valid time.
pub const TM_VALID_FROM: &str = "urn:tiramemsu:tm:validFrom";
/// `e tm:validTo dt`: the bounded end of `e`'s valid time.
pub const TM_VALID_TO: &str = "urn:tiramemsu:tm:validTo";
/// `tm:run`: the subject of the facts about the current run.
pub const TM_RUN: &str = "urn:tiramemsu:tm:run";
/// `tm:run tm:now dt`: the run's pinned clock (the view's instant).
pub const TM_NOW: &str = "urn:tiramemsu:tm:now";
/// `tm:run tm:segmentFrom dt`: the bounded start of the segment being reasoned.
pub const TM_SEGMENT_FROM: &str = "urn:tiramemsu:tm:segmentFrom";
/// `tm:run tm:segmentTo dt`: the bounded end of the segment being reasoned.
pub const TM_SEGMENT_TO: &str = "urn:tiramemsu:tm:segmentTo";

/// `time:localTime`, rewritten to `tm:run tm:now` so it reads the pinned clock.
pub const TIME_LOCAL_TIME: &str = eyeron::ast::TIME_LOCAL_TIME;
/// `log:notIncludes` (negation as failure: non-monotonic).
pub const LOG_NOT_INCLUDES: &str = eyeron::ast::LOG_NOT_INCLUDES;
/// `log:collectAllIn` (aggregation: non-monotonic).
pub const LOG_COLLECT_ALL_IN: &str = eyeron::ast::LOG_COLLECT_ALL_IN;
/// `log:forAllIn` (universal check: non-monotonic).
pub const LOG_FOR_ALL_IN: &str = eyeron::ast::LOG_FOR_ALL_IN;

/// Prefixes every rule text may use without declaring them. The prelude has no
/// newline, so parse errors keep the line numbers of the caller's text.
pub(crate) const PRELUDE: &str = "@prefix v: <urn:tiramemsu:v:> . \
     @prefix tm: <urn:tiramemsu:tm:> . @prefix n3: <urn:tiramemsu:n3:> . ";

/// True for an IRI the reasoner evaluates itself (SWAP `log:`, `math:`,
/// `string:`, `list:`, `time:`, `crypto:`, and eyeron's own datatype and Sudoku
/// extensions): a premise with such a predicate is a computation, not a
/// statement.
pub(crate) fn is_builtin(iri: &str) -> bool {
    use eyeron::ast::{
        DT_DATATYPE, DT_LEXICAL_FORM, EYELING_DT_DATATYPE, EYELING_DT_LEXICAL_FORM, SUDOKU_SOLVE,
    };
    iri.starts_with("http://www.w3.org/2000/10/swap/")
        || iri.starts_with("urn:eyeron:")
        || [
            DT_DATATYPE,
            DT_LEXICAL_FORM,
            EYELING_DT_DATATYPE,
            EYELING_DT_LEXICAL_FORM,
            SUDOKU_SOLVE,
        ]
        .contains(&iri)
}

/// The prefixed name of a builtin the monotonicity check reports.
pub(crate) fn builtin_name(iri: &str) -> String {
    for (ns, p) in [
        ("http://www.w3.org/2000/10/swap/log#", "log:"),
        ("http://www.w3.org/2000/10/swap/time#", "time:"),
        (TM, "tm:"),
    ] {
        if let Some(local) = iri.strip_prefix(ns) {
            return format!("{p}{local}");
        }
    }
    iri.to_string()
}
