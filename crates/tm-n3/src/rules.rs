//! Parsing and analysing a rule set: the predicates its bodies read (for
//! pruning), the virtual predicates it uses, the monotonicity check and the
//! `time:localTime` rewrite.

use std::collections::{BTreeMap, BTreeSet};

use eyeron::{Document, Graph, PreparedReasoner, Rule, Term as NTerm, Triple};
use tm_core::{Error, Result};

use crate::vocab::{
    builtin_name, is_builtin, LOG_COLLECT_ALL_IN, LOG_FOR_ALL_IN, LOG_NOT_INCLUDES, PRELUDE,
    TIME_LOCAL_TIME, TM, TM_CONTENT, TM_NOW, TM_RUN, TM_SEGMENT_FROM, TM_SEGMENT_TO, TM_VALID_FROM,
    TM_VALID_TO,
};

/// Whether a rule set may be materialized (design N11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Monotonicity {
    /// No rule body uses negation, aggregation or the clock.
    Monotonic,
    /// A rule body uses `log:notIncludes`, `log:collectAllIn` or `log:forAllIn`.
    NonMonotonic {
        /// The index of the first such rule.
        rule: usize,
        /// The builtin, as a prefixed name (`log:notIncludes`).
        builtin: String,
    },
    /// A rule body reads the clock (`time:localTime` or `tm:now`).
    ClockDependent {
        /// The index of the first such rule.
        rule: usize,
        /// The builtin, as a prefixed name (`time:localTime`, `tm:now`).
        builtin: String,
    },
    /// A rule body uses a builtin whose proof does not name the facts it read
    /// (`list:map`).
    UntracedPremises {
        /// The index of the first such rule.
        rule: usize,
        /// The builtin, as a prefixed name (`list:map`).
        builtin: String,
    },
}

/// What a rule set is: its classification, size and the predicates its bodies read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleSetInfo {
    /// The monotonicity classification.
    pub monotonicity: Monotonicity,
    /// The number of rules (forward, backward, query and fuse rules, in text order).
    pub rules: usize,
    /// The data predicates in predicate positions of the rule bodies, nested
    /// formulas included, ascending (builtins and `tm:` virtual predicates left out).
    pub predicates: Vec<String>,
    /// True when some rule body has a variable in a predicate position.
    pub variable_predicate: bool,
}

/// Which virtual triples the rules can read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Virtuals {
    pub content: bool,
    pub valid_from: bool,
    pub valid_to: bool,
    /// The `tm:run` facts (`tm:now`, `tm:segmentFrom`, `tm:segmentTo`).
    pub run: bool,
}

/// A parsed rule set, ready to run.
#[derive(Clone, Debug)]
pub struct RuleSet {
    text: String,
    /// The rules as run: `time:localTime` rewritten to `tm:run tm:now`.
    pub(crate) rules: Vec<Rule>,
    pub(crate) program: PreparedReasoner,
    pub(crate) prefixes: BTreeMap<String, String>,
    pub(crate) virtuals: Virtuals,
    /// `None` when every statement of the scope must be loaded.
    load: Option<Vec<String>>,
    info: RuleSetInfo,
}

/// The predicate positions of one rule body.
#[derive(Default)]
struct BodyUse {
    preds: BTreeSet<String>,
    var_pred: bool,
}

impl BodyUse {
    fn graph(&mut self, g: &Graph) {
        self.graph_in(g, false);
    }

    /// `content`: inside the object formula of a `tm:content` triple. Those
    /// formulas are made from statements already loaded, so a variable predicate
    /// there never asks for more statements.
    fn graph_in(&mut self, g: &Graph, content: bool) {
        for t in &g.triples {
            match &t.p {
                NTerm::Iri(p) => {
                    self.preds.insert(p.to_string());
                }
                NTerm::Var(_) | NTerm::Blank(_) if !content => self.var_pred = true,
                _ => {}
            }
            // `((items) pred) list:map ?out` applies `pred` as a predicate
            if matches!(&t.p, NTerm::Iri(p) if p == eyeron::ast::LIST_MAP) {
                if let NTerm::List(args) = &t.s {
                    match args.get(1) {
                        Some(NTerm::Iri(p)) => {
                            self.preds.insert(p.to_string());
                        }
                        Some(NTerm::Var(_) | NTerm::Blank(_)) => self.var_pred = true,
                        _ => {}
                    }
                }
            }
            let of_content = matches!(&t.p, NTerm::Iri(p) if p == TM_CONTENT);
            self.term_in(&t.s, content);
            self.term_in(&t.p, content);
            self.term_in(&t.o, content || of_content);
        }
    }

    /// Nested formulas (and formulas in lists) of a term.
    fn term(&mut self, t: &NTerm) {
        self.term_in(t, false);
    }

    fn term_in(&mut self, t: &NTerm, content: bool) {
        match t {
            NTerm::Formula(g) => self.graph_in(g, content),
            NTerm::List(items) => items.iter().for_each(|i| self.term_in(i, content)),
            _ => {}
        }
    }
}

/// `time:localTime` premises read the run's pinned clock (design N7).
fn rewrite_graph(g: &Graph) -> Graph {
    g.map_triples(|t| {
        if matches!(&t.p, NTerm::Iri(p) if p == TIME_LOCAL_TIME) {
            Triple::new(NTerm::iri(TM_RUN), NTerm::iri(TM_NOW), rewrite_term(&t.o))
        } else {
            Triple::new(rewrite_term(&t.s), rewrite_term(&t.p), rewrite_term(&t.o))
        }
    })
}

fn rewrite_term(t: &NTerm) -> NTerm {
    match t {
        NTerm::Formula(g) => NTerm::Formula(rewrite_graph(g)),
        NTerm::List(items) => NTerm::List(items.iter().map(rewrite_term).collect()),
        other => other.clone(),
    }
}

/// The 1-based line of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    1 + text
        .as_bytes()
        .iter()
        .take(offset)
        .filter(|&&b| b == b'\n')
        .count()
}

impl RuleSet {
    /// Parses N3 rule text. The prefixes `v:` (`urn:tiramemsu:v:`), `tm:` and
    /// `n3:` are predeclared, as are eyeron's (`rdf:`, `rdfs:`, `xsd:`, `log:`,
    /// `math:`, `string:`, `list:`, `time:`, ...). Facts in the text are part of
    /// the rule set.
    ///
    /// # Errors
    ///
    /// `N3Parse { line, message }` for text that does not parse.
    // @tg: implements:: [[openspec:n3-rule-sets#Monotonicity check]] {since: "2026-10-28"}
    pub fn parse(n3: &str) -> Result<RuleSet> {
        let full = format!("{PRELUDE}{n3}");
        let doc = eyeron::parse_n3(&full, None).map_err(|e| Error::N3Parse {
            line: e.offset.map_or(0, |o| line_of(&full, o)),
            message: e.message.clone(),
        })?;
        Ok(RuleSet::from_document(n3.to_string(), doc))
    }

    fn from_document(text: String, doc: Document) -> RuleSet {
        let mut preds = BTreeSet::new();
        let mut variable_predicate = false;
        let mut prune = true;
        let mut virtuals = Virtuals::default();
        let mut non_monotonic = None;
        let mut clock = None;
        for (i, rule) in doc.rules.iter().enumerate() {
            let mut body = BodyUse::default();
            body.graph(&rule.premise);
            // formulas a rule concludes may be rules themselves
            for t in &rule.conclusion.triples {
                for term in [&t.s, &t.p, &t.o] {
                    body.term(term);
                }
            }
            variable_predicate |= body.var_pred;
            let has = |p: &str| body.preds.contains(p);
            virtuals.content |= has(TM_CONTENT);
            virtuals.valid_from |= has(TM_VALID_FROM);
            virtuals.valid_to |= has(TM_VALID_TO);
            virtuals.run |= [TIME_LOCAL_TIME, TM_NOW, TM_SEGMENT_FROM, TM_SEGMENT_TO]
                .into_iter()
                .any(has);
            // validity read about a statement whose content the rule does not
            // pin down: any statement may be meant
            if (has(TM_VALID_FROM) || has(TM_VALID_TO)) && !has(TM_CONTENT) {
                prune = false;
            }
            if non_monotonic.is_none() {
                if let Some(b) = [LOG_NOT_INCLUDES, LOG_COLLECT_ALL_IN, LOG_FOR_ALL_IN]
                    .into_iter()
                    .find(|b| has(b))
                {
                    non_monotonic = Some((i, builtin_name(b)));
                }
            }
            if clock.is_none() {
                if let Some(b) = [TIME_LOCAL_TIME, TM_NOW].into_iter().find(|b| has(b)) {
                    clock = Some((i, builtin_name(b)));
                }
            }
            preds.extend(
                body.preds
                    .into_iter()
                    .filter(|p| !is_builtin(p) && !p.starts_with(TM)),
            );
        }
        // a variable predicate may match any statement's virtual triples (the
        // `tm:run` facts only when a body names them)
        if variable_predicate {
            virtuals.content = true;
            virtuals.valid_from = true;
            virtuals.valid_to = true;
        }
        let monotonicity = match (non_monotonic, clock) {
            (Some((rule, builtin)), _) => Monotonicity::NonMonotonic { rule, builtin },
            (None, Some((rule, builtin))) => Monotonicity::ClockDependent { rule, builtin },
            (None, None) => Monotonicity::Monotonic,
        };
        let predicates: Vec<String> = preds.into_iter().collect();
        let info = RuleSetInfo {
            monotonicity,
            rules: doc.rules.len(),
            predicates: predicates.clone(),
            variable_predicate,
        };
        let rules: Vec<Rule> = doc
            .rules
            .iter()
            .map(|r| Rule {
                premise: rewrite_graph(&r.premise),
                ..r.clone()
            })
            .collect();
        let prefixes = doc.prefixes.clone();
        let program = PreparedReasoner::new(Document {
            rules: rules.clone(),
            ..doc
        });
        RuleSet {
            text,
            rules,
            program,
            prefixes,
            virtuals,
            load: (prune && !variable_predicate).then_some(predicates),
            info,
        }
    }

    /// The text this rule set was parsed from.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The classification, rule count and body predicates.
    pub fn info(&self) -> &RuleSetInfo {
        &self.info
    }

    /// The predicates whose statements a run must load, or `None` when it must
    /// load every statement of its scope (a variable predicate, or validity read
    /// about statements of any content). This is the pruning of design N4.
    // @tg: implements:: [[openspec:n3-reasoning#Reasoning scope and pruning]] {since: "2026-10-28"}
    pub fn load_predicates(&self) -> Option<&[String]> {
        self.load.as_deref()
    }

    /// True when the rules read the virtual triples of statements (`tm:content`,
    /// `tm:validFrom`, `tm:validTo`): a loader then also loads the statements
    /// the loaded ones are about (layers' targets), so those triples exist.
    pub fn reads_statements(&self) -> bool {
        let v = self.virtuals;
        v.content || v.valid_from || v.valid_to
    }

    /// The index of `rule` in this rule set, if it is one of its rules.
    pub(crate) fn index_of(&self, rule: &Rule) -> Option<usize> {
        self.rules.iter().position(|r| r == rule).or_else(|| {
            let src = rule.source.as_ref()?;
            self.rules
                .iter()
                .position(|r| r.source.as_ref() == Some(src))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negation_is_detected() {
        let rs = RuleSet::parse(
            "{ ?x a v:Employee. ?SCOPE log:notIncludes { ?x v:manager ?m } } => { ?x a v:Unmanaged } .",
        )
        .unwrap();
        assert_eq!(
            rs.info().monotonicity,
            Monotonicity::NonMonotonic {
                rule: 0,
                builtin: "log:notIncludes".into()
            }
        );
        // the nested formula's predicate is read too
        assert_eq!(
            rs.load_predicates().unwrap(),
            [
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
                "urn:tiramemsu:v:manager"
            ]
        );
    }

    #[test]
    fn clock_and_monotonic() {
        let rs = RuleSet::parse(
            "{ ?x a v:A } => { ?x a v:B } .\n{ \"\" time:localTime ?t } => { v:run v:at ?t } .",
        )
        .unwrap();
        assert_eq!(
            rs.info().monotonicity,
            Monotonicity::ClockDependent {
                rule: 1,
                builtin: "time:localTime".into()
            }
        );
        // rewritten to the pinned clock
        assert_eq!(rs.rules[1].premise.triples[0].p, NTerm::iri(TM_NOW));
        let rs = RuleSet::parse("{ ?x a v:A } => { ?x a v:B } .").unwrap();
        assert_eq!(rs.info().monotonicity, Monotonicity::Monotonic);
        assert_eq!(rs.info().rules, 1);
    }

    #[test]
    fn backward_rules_and_variable_predicates() {
        let rs =
            RuleSet::parse("{ ?x v:r ?y } <= { ?x v:q ?y } . { ?x v:r ?y } => { ?x v:s ?y } .")
                .unwrap();
        assert_eq!(
            rs.load_predicates().unwrap(),
            ["urn:tiramemsu:v:q", "urn:tiramemsu:v:r"]
        );
        let rs = RuleSet::parse("{ ?s ?p ?o } => { ?s v:seen true } .").unwrap();
        assert!(rs.load_predicates().is_none());
        assert!(rs.info().variable_predicate);
        // a variable predicate inside `tm:content` keeps pruning on
        let rs = RuleSet::parse(
            "{ ?e v:confidence ?c. ?e tm:content { ?s ?p ?o } } => { ?s v:trusted ?o } .",
        )
        .unwrap();
        assert_eq!(
            rs.load_predicates().unwrap(),
            ["urn:tiramemsu:v:confidence"]
        );
        assert!(rs.reads_statements());
        // but not inside other formulas
        let rs = RuleSet::parse("{ ?g log:includes { ?s ?p ?o } } => { ?g v:has ?s } .").unwrap();
        assert!(rs.load_predicates().is_none());
    }

    #[test]
    fn parse_errors_have_lines() {
        match RuleSet::parse("{ ?x a v:Human } =>\n\n ") {
            Err(Error::N3Parse { line, .. }) => assert_eq!(line, 3),
            other => panic!("{other:?}"),
        }
    }
}
