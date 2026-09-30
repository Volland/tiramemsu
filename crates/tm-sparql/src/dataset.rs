//! Time IRIs and view scopes (`lat.md/query#Temporal Syntax`, design D7).
//!
//! `FROM` / `USING` set the query default, `SERVICE <tm:…> { … }` scopes one
//! group. A [`ViewScope`] holds the transaction-time and valid-time parts that a
//! clause names; parts are resolved innermost first against the caller's view.

// @lat: [[query#Temporal Syntax]]

use spargebra::algebra::QueryDataset;
use spargebra::term::NamedNode;
use tm_core::{value, vocab, Error, Result, TimeRef, TxSel, ValidSel, Value};
use tm_ir::View;

use crate::error::{invalid_time_iri, unsupported, CONFLICTING_TIME};

/// A parsed `tm:` time IRI.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TimeIri {
    /// `asOf/<t>`, `asOf/<instant>` or `history`: the transaction-time part.
    Tx(TxSel),
    /// `validAt/<instant>`: the valid-time part.
    Valid(ValidSel),
}

/// True when `iri` is in the `tm:` namespace.
pub fn is_tm_iri(iri: &str) -> bool {
    iri.starts_with(vocab::TM)
}

/// An instant given as an `xsd:date` or `xsd:dateTime` lexical form, in epoch
/// milliseconds. A date is 00:00:00 UTC, a date-time without a timezone is UTC.
fn instant_ms(text: &str) -> Option<i64> {
    if text.contains('T') {
        value::parse_datetime(text).map(|(ms, _)| ms)
    } else {
        value::parse_date(text).map(|d| d * 86_400_000)
    }
}

/// Parses `iri` as a time IRI. `Ok(None)` when it is not in the `tm:` namespace;
/// a `tm:` IRI that is not a valid time IRI is a `Parse` error naming the IRI.
pub fn parse_time_iri(iri: &str) -> Result<Option<TimeIri>> {
    let Some(rest) = iri.strip_prefix(vocab::TM) else {
        return Ok(None);
    };
    let bad = || invalid_time_iri(iri);
    if rest == "history" {
        return Ok(Some(TimeIri::Tx(TxSel::History)));
    }
    if let Some(a) = rest.strip_prefix("asOf/") {
        if !a.is_empty() && a.bytes().all(|b| b.is_ascii_digit()) {
            let t: u64 = a.parse().map_err(|_| bad())?;
            return Ok(Some(TimeIri::Tx(TxSel::AsOf(TimeRef::Tx(t)))));
        }
        let ms = instant_ms(a).ok_or_else(bad)?;
        return Ok(Some(TimeIri::Tx(TxSel::AsOf(TimeRef::Instant(ms)))));
    }
    if let Some(a) = rest.strip_prefix("validAt/") {
        let ms = instant_ms(a).ok_or_else(bad)?;
        return Ok(Some(TimeIri::Valid(ValidSel::At(ms))));
    }
    Err(bad())
}

/// The time parts a clause names; an absent part is inherited.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewScope {
    /// The transaction-time part.
    pub tx: Option<TxSel>,
    /// The valid-time part.
    pub valid: Option<ValidSel>,
}

impl ViewScope {
    /// A scope that names nothing (the caller's view applies unchanged).
    pub fn inherit() -> ViewScope {
        ViewScope::default()
    }

    /// This scope with the part named by `iri` replaced (a nested `SERVICE`).
    pub fn nested(self, iri: TimeIri) -> ViewScope {
        match iri {
            TimeIri::Tx(t) => ViewScope {
                tx: Some(t),
                ..self
            },
            TimeIri::Valid(v) => ViewScope {
                valid: Some(v),
                ..self
            },
        }
    }

    /// Adds `iri` as a sibling clause (`FROM`): the same part twice with different
    /// values is `Unsupported("conflicting time selectors")`.
    fn add_clause(&mut self, iri: TimeIri) -> Result<()> {
        match iri {
            TimeIri::Tx(t) => match self.tx {
                Some(old) if old != t => return Err(unsupported(CONFLICTING_TIME)),
                _ => self.tx = Some(t),
            },
            TimeIri::Valid(v) => match self.valid {
                Some(old) if old != v => return Err(unsupported(CONFLICTING_TIME)),
                _ => self.valid = Some(v),
            },
        }
        Ok(())
    }

    /// The time scope of a `FROM` / `USING` dataset: the `tm:` IRIs of the default
    /// clauses. `FROM NAMED` / `USING NAMED` time IRIs are validated and have no
    /// effect; every other IRI names a graph and is read by [`GraphDataset`].
    pub fn from_dataset(ds: Option<&QueryDataset>) -> Result<ViewScope> {
        let mut scope = ViewScope::default();
        let Some(ds) = ds else { return Ok(scope) };
        for n in &ds.default {
            if let Some(t) = parse_time_iri(n.as_str())? {
                scope.add_clause(t)?;
            }
        }
        for n in ds.named.iter().flatten() {
            parse_time_iri(n.as_str())?;
        }
        Ok(scope)
    }

    /// The scope of a `SERVICE` endpoint below `self`: `Ok(None)` when the IRI is
    /// not a time IRI (federation).
    pub fn enter_service(self, name: &NamedNode) -> Result<Option<ViewScope>> {
        Ok(parse_time_iri(name.as_str())?.map(|t| self.nested(t)))
    }

    /// The view a pattern in this scope reads: the parts named here, the rest
    /// from `base` (the view the text was submitted on).
    pub fn resolve(self, base: View) -> View {
        base.overlay(self.tx, self.valid)
    }
}

/// A graph name as a value: an IRI, `NODE` or `BNODE`. Any other term (a statement
/// or transaction skolem IRI) fails with `InvalidGraphName`.
pub fn graph_name(iri: &str) -> Result<Value> {
    let v = Value::Iri(iri.to_string()).canonical();
    match v {
        Value::Iri(_) | Value::Node(_) | Value::BNode(_) => Ok(v),
        other => Err(Error::InvalidGraphName {
            term: other.to_string(),
        }),
    }
}

/// The graph part of a `FROM` / `USING` dataset (`lat.md/data-model#Named Graphs`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GraphDataset {
    /// The non-time IRIs of `FROM` / `USING`: the default graph is the statements in
    /// at least one of them. Empty: the default graph is the union of everything.
    pub default: Vec<Value>,
    /// The non-time IRIs of `FROM NAMED` / `USING NAMED`. `Some` restricts `GRAPH`
    /// to them; `None` means every graph is visible.
    pub named: Option<Vec<Value>>,
}

impl GraphDataset {
    /// The graph part of `ds`; `tm:` IRIs are skipped (see [`ViewScope`]).
    pub fn from_dataset(ds: Option<&QueryDataset>) -> Result<GraphDataset> {
        let mut out = GraphDataset::default();
        let Some(ds) = ds else { return Ok(out) };
        for n in &ds.default {
            if !is_tm_iri(n.as_str()) {
                let g = graph_name(n.as_str())?;
                if !out.default.contains(&g) {
                    out.default.push(g);
                }
            }
        }
        for n in ds.named.iter().flatten() {
            if !is_tm_iri(n.as_str()) {
                let g = graph_name(n.as_str())?;
                let list = out.named.get_or_insert_with(Vec::new);
                if !list.contains(&g) {
                    list.push(g);
                }
            }
        }
        Ok(out)
    }

    /// The selector of the default graph.
    pub fn default_selector(&self) -> tm_ir::GraphSel {
        if self.default.is_empty() {
            tm_ir::GraphSel::Any
        } else {
            tm_ir::GraphSel::Set(
                self.default
                    .iter()
                    .cloned()
                    .map(tm_ir::TermOrVar::Const)
                    .collect(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tm_core::Error;

    const P: &str = "urn:tiramemsu:tm:";

    fn ok(s: &str) -> TimeIri {
        parse_time_iri(&format!("{P}{s}")).unwrap().unwrap()
    }

    fn ms(text: &str) -> i64 {
        value::parse_datetime(text).unwrap().0
    }

    // sparql-temporal-dataset "Time IRI grammar": each row of the D7 table
    #[test]
    fn grammar_rows() {
        assert_eq!(ok("asOf/5"), TimeIri::Tx(TxSel::AsOf(TimeRef::Tx(5))));
        assert_eq!(
            ok("asOf/2026-09-01T12:00:00Z"),
            TimeIri::Tx(TxSel::AsOf(TimeRef::Instant(ms("2026-09-01T12:00:00Z"))))
        );
        // date: 00:00Z; no timezone: UTC; offset resolves to the instant
        assert_eq!(ok("asOf/2026-09-01"), ok("asOf/2026-09-01T00:00:00Z"));
        assert_eq!(
            ok("asOf/2026-09-01T12:00:00"),
            ok("asOf/2026-09-01T12:00:00Z")
        );
        assert_eq!(
            ok("asOf/2026-09-01T14:00:00+02:00"),
            ok("asOf/2026-09-01T12:00:00Z")
        );
        assert_eq!(
            ok("validAt/2025-03-01"),
            TimeIri::Valid(ValidSel::At(ms("2025-03-01T00:00:00Z")))
        );
        assert_eq!(ok("history"), TimeIri::Tx(TxSel::History));
    }

    #[test]
    fn invalid_and_foreign_iris() {
        for bad in [
            "asOf/yesterday",
            "asOf/",
            "validAt/5",
            "txAdded",
            "history/1",
            "asOf/-3",
        ] {
            let e = parse_time_iri(&format!("{P}{bad}")).unwrap_err();
            assert!(
                matches!(&e, Error::Parse { msg, .. } if msg.contains(&format!("{P}{bad}"))),
                "{bad}: {e:?}"
            );
        }
        assert_eq!(parse_time_iri("http://example.org/x").unwrap(), None);
    }

    fn ds(default: &[&str], named: Option<&[&str]>) -> QueryDataset {
        QueryDataset {
            default: default
                .iter()
                .map(|s| NamedNode::new_unchecked(*s))
                .collect(),
            named: named.map(|n| n.iter().map(|s| NamedNode::new_unchecked(*s)).collect()),
        }
    }

    // sparql-temporal-dataset "FROM sets the query default view"
    #[test]
    fn from_combines_parts_and_overrides_the_base_view() {
        let d = ds(
            &[&format!("{P}asOf/150"), &format!("{P}validAt/2025-03-01")],
            None,
        );
        let s = ViewScope::from_dataset(Some(&d)).unwrap();
        let base = View::now().valid_at(1);
        let v = s.resolve(base);
        assert_eq!(v.tx, TxSel::AsOf(TimeRef::Tx(150)));
        assert_eq!(v.valid, ValidSel::At(ms("2025-03-01T00:00:00Z")));
        // a part FROM does not name is inherited
        let only_tx = ViewScope::from_dataset(Some(&ds(&[&format!("{P}asOf/100")], None))).unwrap();
        let v = only_tx.resolve(View::as_of_tx(200).valid_at(ms("2025-01-01T00:00:00Z")));
        assert_eq!(v.tx, TxSel::AsOf(TimeRef::Tx(100)));
        assert_eq!(v.valid, ValidSel::At(ms("2025-01-01T00:00:00Z")));
    }

    #[test]
    fn conflicting_selectors_and_repeats() {
        let d = ds(&[&format!("{P}asOf/5"), &format!("{P}history")], None);
        assert!(matches!(
            ViewScope::from_dataset(Some(&d)),
            Err(Error::Unsupported { feature }) if feature == CONFLICTING_TIME
        ));
        let same = ds(&[&format!("{P}asOf/5"), &format!("{P}asOf/5")], None);
        assert!(ViewScope::from_dataset(Some(&same)).is_ok());
    }

    #[test]
    fn graph_iris_are_not_time_and_time_iris_are_not_graphs() {
        let g = ds(
            &["http://example.org/graph1"],
            Some(&["http://example.org/g2"]),
        );
        assert_eq!(
            ViewScope::from_dataset(Some(&g)).unwrap(),
            ViewScope::inherit()
        );
        let gd = GraphDataset::from_dataset(Some(&g)).unwrap();
        assert_eq!(gd.default, vec![Value::iri("http://example.org/graph1")]);
        assert_eq!(gd.named, Some(vec![Value::iri("http://example.org/g2")]));
        // a time IRI beside a graph IRI: each side reads its own part
        let both = ds(&[&format!("{P}asOf/7"), "http://example.org/g1"], None);
        assert!(ViewScope::from_dataset(Some(&both)).unwrap().tx.is_some());
        assert_eq!(
            GraphDataset::from_dataset(Some(&both))
                .unwrap()
                .default
                .len(),
            1
        );
        // FROM NAMED with a time IRI is a no-op
        let n = ds(&[], Some(&[&format!("{P}asOf/9")]));
        assert_eq!(
            ViewScope::from_dataset(Some(&n)).unwrap(),
            ViewScope::inherit()
        );
        assert_eq!(
            GraphDataset::from_dataset(Some(&n)).unwrap(),
            GraphDataset::default()
        );
        // a statement IRI is not a graph name
        let bad = ds(&["urn:tiramemsu:stmt:5"], None);
        assert!(matches!(
            GraphDataset::from_dataset(Some(&bad)),
            Err(Error::InvalidGraphName { .. })
        ));
    }

    // sparql-temporal-dataset "SERVICE scopes a group in time": nesting
    #[test]
    fn nested_scopes_combine_and_innermost_wins() {
        let base = ViewScope::from_dataset(Some(&ds(&[&format!("{P}asOf/10")], None))).unwrap();
        let s1 = base.nested(ok("asOf/150"));
        let s2 = s1.nested(ok("validAt/2021-06-01"));
        let v = s2.resolve(View::NOW);
        assert_eq!(v.tx, TxSel::AsOf(TimeRef::Tx(150)));
        assert_eq!(v.valid, ValidSel::At(ms("2021-06-01T00:00:00Z")));
        let s3 = s1.nested(ok("asOf/90"));
        assert_eq!(s3.resolve(View::NOW).tx, TxSel::AsOf(TimeRef::Tx(90)));
        // inner scope overrides FROM
        let h = base.nested(ok("history"));
        assert_eq!(h.resolve(View::NOW).tx, TxSel::History);
        // a service IRI outside tm: is not a time scope
        assert_eq!(
            base.enter_service(&NamedNode::new_unchecked("http://dbpedia.org/sparql"))
                .unwrap(),
            None
        );
    }
}
