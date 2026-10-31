# tm-n3

N3 reasoning for [Tiramemsu](https://github.com/Volland/tiramemsu) with
[eyeron](https://github.com/eyereasoner/eyeron), aware of valid and
transaction time (OpenSpec change `add-n3-reasoning`).

This crate is pure: it takes loaded statements (eid, subject, predicate,
object as `tm_core::Value`, valid interval), rule text and options, and
returns conclusions with proofs. It never reads or writes a store. The
`tiramemsu` facade loads a scope of a view and calls it behind its `n3`
feature (off by default): `View::reason`, `View::rule_set_info` and
`TxN3::store_rules`.

- **Statements in N3.** Values map through the RDF rendering of fact bundles:
  IRIs and literals as themselves, `urn:tiramemsu:node|bnode|stmt:N` for
  anonymous nodes, blank nodes and statements. A rule body may read the
  virtual triples `e tm:content { s p o }`, `e tm:validFrom dt` and
  `e tm:validTo dt` of a loaded statement `e`, and `tm:run tm:now dt`.
- **Pointwise valid time.** A conclusion holds at `t` iff it follows from the
  statements valid at `t`: the window is cut at every valid-time bound, each
  segment is reasoned once over the statements valid throughout it, and
  adjacent segments with the same conclusion and support merge.
- **Pinned clock.** `time:localTime` and `tm:run tm:now` yield the run's
  `now`, never the wall clock.
- **Proofs.** Every conclusion names its rule and its premises: loaded
  statement eids and earlier conclusions.
- **Monotonicity.** A rule set using `log:notIncludes`, `log:collectAllIn` or
  `log:forAllIn` is non-monotonic; one using `time:localTime` or `tm:now` is
  clock-dependent.

```rust
use tm_core::{vocab, Eid, Valid, Value};
use tm_n3::{reason, Fact, Input, Monotonicity, Premise, ReasonOptions, RuleSet};

let v = |s: &str| Value::iri(format!("urn:tiramemsu:v:{s}"));
// `v:` is predeclared as `urn:tiramemsu:v:`
let rules = RuleSet::parse(
    "{ ?p v:worksAt ?c. ?c v:locatedIn ?l } => { ?p v:worksIn ?l } .",
)?;
assert_eq!(rules.info().monotonicity, Monotonicity::Monotonic);

let facts = [
    Fact { eid: Eid::new(1), s: v("alice"), p: v("worksAt"), o: v("acme"),
           valid: Valid::between(2020, 2025) },
    Fact { eid: Eid::new(2), s: v("acme"), p: v("locatedIn"), o: v("berlin"),
           valid: Valid::from(2024) },
];
let r = reason(&rules, &Input::new(&facts, 0), &ReasonOptions::default())?;
assert_eq!(r.conclusions.len(), 1);
let c = &r.conclusions[0];
assert_eq!(c.values().unwrap().2, v("berlin"));
assert_eq!(c.valid, Valid::between(2024, 2025)); // only while both held
assert_eq!(c.proof.premises, [Premise::Statement(Eid::new(1)), Premise::Statement(Eid::new(2))]);
# let _ = vocab::RDF_TYPE;
# Ok::<(), tm_core::Error>(())
```

eyeron is MIT-licensed (Jos De Roo). It is used from crates.io with
`default-features = false`, which leaves out its HTTP client.
