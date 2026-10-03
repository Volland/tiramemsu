//! The SPARQL time-respecting path scope (`add-temporal-path-syntax`): `SERVICE
//! <urn:tiramemsu:tm:timeRespecting[/<t>]>` and `?end tm:arrival ?t` lower to the
//! shared IR's `TemporalPath` on the path pattern.
mod common;
use common::*;
use tm_core::Error;

const TR: &str = "urn:tiramemsu:tm:timeRespecting";

// @lat: [[tests#Temporal Path Syntax#SPARQL Scope Lowers To The Shared IR]]
#[test]
fn the_scope_lowers_to_the_temporal_path_pattern() {
    // from −∞, no arrival: only the modifier
    let s = ir(&format!(
        "SELECT ?y WHERE {{ SERVICE <{TR}> {{ v:a v:met+ ?y }} }}"
    ));
    assert!(s.contains(":mode REACH"), "{s}");
    assert!(s.contains(":timeRespecting"), "{s}");
    assert!(!s.contains(":after") && !s.contains(":arrival"), "{s}");
    // a literal start and the arrival binding; no triple pattern for tm:arrival
    let s = ir(&format!(
        "SELECT ?y ?t WHERE {{ SERVICE <{TR}/1717200000000> {{ v:a v:met+ ?y . ?y tm:arrival ?t }} }}"
    ));
    assert!(
        s.contains(":timeRespecting :after 1717200000000 :arrival ?t"),
        "{s}"
    );
    assert!(!s.contains("arrival>"), "{s}");
    // a date start is its instant; a parameter stays a parameter
    let s = ir(&format!(
        "SELECT ?y WHERE {{ SERVICE <{TR}/2024-06-01> {{ v:a v:met+ ?y }} }}"
    ));
    assert!(s.contains(":after 1717200000000"), "{s}");
    let s = ir(&format!(
        "SELECT ?y WHERE {{ SERVICE <{TR}/$start> {{ v:a v:met+ ?y }} }}"
    ));
    assert!(s.contains(":after $start"), "{s}");
    // inherited by a nested time scope, which still selects the view
    let s = ir(&format!(
        "SELECT ?y WHERE {{ SERVICE <{TR}> {{ SERVICE <urn:tiramemsu:tm:asOf/3> {{ v:a v:met+ ?y }} }} }}"
    ));
    assert!(s.contains(":timeRespecting") && s.contains("asOf"), "{s}");
    // a non-recursive path in the scope is a time-respecting region too
    let s = ir(&format!(
        "SELECT ?y WHERE {{ SERVICE <{TR}> {{ v:a (v:met|v:knows) ?y }} }}"
    ));
    assert!(s.contains("(path") && s.contains(":timeRespecting"), "{s}");
    // outside the scope nothing changes
    let s = ir("SELECT ?y WHERE { v:a v:met+ ?y }");
    assert!(!s.contains(":timeRespecting"), "{s}");
    let s = ir("SELECT ?y WHERE { v:a v:met|v:knows ?y }");
    assert!(!s.contains("(path"), "{s}");
}

// @lat: [[tests#Temporal Path Syntax#SPARQL Scope Grammar]]
#[test]
fn malformed_scopes_are_parse_errors() {
    let parse = |q: &str| matches!(err(q), Error::Parse { .. });
    for bad in [
        format!("SELECT ?y WHERE {{ SERVICE <{TR}/soon> {{ v:a v:met+ ?y }} }}"),
        format!("SELECT ?y WHERE {{ SERVICE <{TR}/$> {{ v:a v:met+ ?y }} }}"),
        format!("SELECT ?y WHERE {{ SERVICE <{TR}/$1x> {{ v:a v:met+ ?y }} }}"),
        format!("SELECT ?y FROM <{TR}> WHERE {{ v:a v:met+ ?y }}"),
        format!("SELECT ?y WHERE {{ GRAPH <{TR}> {{ v:a v:met+ ?y }} }}"),
        format!("SELECT ?y WHERE {{ SERVICE <{TR}> {{ v:a v:met ?y }} }}"),
        "SELECT ?y ?t WHERE { v:a v:met+ ?y . ?y tm:arrival ?t }".to_string(),
        format!("SELECT ?y ?t WHERE {{ SERVICE <{TR}> {{ v:a v:met+ ?y . ?z tm:arrival ?t }} }}"),
        format!(
            "SELECT ?y ?t WHERE {{ SERVICE <{TR}> {{ v:a v:met+ ?y . v:b v:met+ ?y . ?y tm:arrival ?t }} }}"
        ),
        format!(
            "SELECT ?y WHERE {{ SERVICE <{TR}> {{ v:a v:met+ ?y . ?y tm:arrival ?t . ?y tm:arrival ?t }} }}"
        ),
    ] {
        assert!(parse(&bad), "{bad}");
    }
    // `timeRespectingly` is no modifier: an invalid time IRI
    assert!(parse(&format!(
        "SELECT ?y WHERE {{ SERVICE <{TR}ly> {{ v:a v:met+ ?y }} }}"
    )));
}
