//! Terms between tiramemsu values and ELPI: the prelude's constructors, as a
//! tree that mirrors lean-elpi's JSON term (`lat.md/elpi#Structured Terms`).

use tm_core::{Eid, TxId, Valid, Value};

/// An ELPI term as lean-elpi passes it: constants, applications of a
/// constant, native values, unassigned variables and anything else printed.
#[derive(Clone, Debug, PartialEq)]
pub enum Tree {
    /// A constant.
    Const(String),
    /// A constant applied to arguments.
    App(String, Vec<Tree>),
    /// An integer.
    Int(i64),
    /// A float.
    Float(f64),
    /// A string.
    Str(String),
    /// An unassigned variable.
    Var(String),
    /// Anything else, printed (lambdas, applied variables).
    Other(String),
}

fn app(c: &str, args: Vec<Tree>) -> Tree {
    Tree::App(c.to_string(), args)
}

fn s(x: &str) -> Tree {
    Tree::Str(x.to_string())
}

/// A value of a solution: a tiramemsu value, or a term that is not one.
#[derive(Clone, Debug, PartialEq)]
pub enum ElpiTerm {
    /// A value.
    Value(Value),
    /// A term no value holds (a variable, a lambda, a list), printed.
    Other(String),
}

/// The term of a value.
// @tg: implements:: [[openspec:elpi-engine#Terms cross both ways]] {since: "2026-10-31"}
// @tg: implements:: [[elpi#Structured Terms]] {since: "2026-10-31"}
pub fn from_value(v: &Value) -> Tree {
    match v {
        Value::Iri(i) => app("iri", vec![s(i)]),
        Value::Str(x) => app("str", vec![s(x)]),
        Value::Int(i) => app("num", vec![Tree::Int(*i)]),
        Value::Double(x) => app("dbl", vec![Tree::Float(*x)]),
        Value::Bool(b) => app("bool", vec![s(if *b { "true" } else { "false" })]),
        Value::LangStr { lex, lang } => app("lang", vec![s(lex), s(lang)]),
        Value::Node(n) => app("node", vec![Tree::Int(*n as i64)]),
        Value::BNode(n) => app("bnode", vec![Tree::Int(*n as i64)]),
        Value::Tx(t) => app("tx", vec![Tree::Int(t.0 as i64)]),
        Value::Stmt(e) => app("stmtv", vec![eid(*e)]),
        other => app(
            "lit",
            vec![s(&other.lexical()), s(other.datatype().unwrap_or(""))],
        ),
    }
}

/// The value of a term, when it is one of the prelude's value constructors
/// with ground arguments.
pub fn to_value(t: &Tree) -> Option<Value> {
    let Tree::App(c, a) = t else {
        return None;
    };
    let str_at = |i: usize| match a.get(i) {
        Some(Tree::Str(x)) => Some(x.clone()),
        _ => None,
    };
    let int_at = |i: usize| match a.get(i) {
        Some(Tree::Int(n)) => Some(*n),
        _ => None,
    };
    Some(match (c.as_str(), a.len()) {
        ("iri", 1) => Value::Iri(str_at(0)?),
        ("str", 1) => Value::Str(str_at(0)?),
        ("num", 1) => Value::Int(int_at(0)?),
        ("dbl", 1) => match a.first() {
            Some(Tree::Float(x)) => Value::Double(*x),
            Some(Tree::Int(n)) => Value::Double(*n as f64),
            _ => return None,
        },
        ("bool", 1) => match str_at(0)?.as_str() {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return None,
        },
        ("lang", 2) => Value::literal(&str_at(0)?, None, Some(&str_at(1)?)),
        ("lit", 2) => Value::literal(&str_at(0)?, Some(&str_at(1)?), None),
        ("node", 1) => Value::Node(u64::try_from(int_at(0)?).ok()?),
        ("bnode", 1) => Value::BNode(u64::try_from(int_at(0)?).ok()?),
        ("tx", 1) => Value::Tx(TxId(u64::try_from(int_at(0)?).ok()?)),
        ("stmtv", 1) => Value::Stmt(to_eid(&a[0])?),
        _ => return None,
    })
}

/// The term `(eid N)` of a statement.
pub fn eid(e: Eid) -> Tree {
    app("eid", vec![Tree::Int(e.n() as i64)])
}

/// The statement of an `(eid N)` term.
pub fn to_eid(t: &Tree) -> Option<Eid> {
    match t {
        Tree::App(c, a) if c == "eid" && a.len() == 1 => match a[0] {
            Tree::Int(n) if n > 0 => Some(Eid::new(n as u64)),
            _ => None,
        },
        _ => None,
    }
}

/// The two bound terms of an interval: `(at Ms)` or `inf`.
pub fn bounds(v: Valid) -> (Tree, Tree) {
    let b = |x: Option<i64>| match x {
        Some(ms) => app("at", vec![Tree::Int(ms)]),
        None => Tree::Const("inf".into()),
    };
    (b(v.from), b(v.to))
}

/// The elements of an ELPI list term (`[]` and `::`, or a printed list).
pub fn list(t: &Tree) -> Option<Vec<Tree>> {
    let mut out = Vec::new();
    let mut cur = t;
    loop {
        match cur {
            Tree::Const(c) if c == "[]" || c == "nil" => return Some(out),
            Tree::App(c, a) if (c == "::" || c == "cons") && a.len() == 2 => {
                out.push(a[0].clone());
                cur = &a[1];
            }
            _ => return None,
        }
    }
}

/// A solution's value: a value when the term is one, else its printed form.
pub fn to_elpi_term(t: &Tree, printed: Option<&str>) -> ElpiTerm {
    match to_value(t) {
        Some(v) => ElpiTerm::Value(v),
        None => ElpiTerm::Other(printed.map_or_else(|| render(t), str::to_string)),
    }
}

/// A term in ELPI's own notation.
pub fn render(t: &Tree) -> String {
    match t {
        Tree::Const(c) => c.clone(),
        Tree::App(c, a) => {
            if let Some(items) = list(t) {
                let inner: Vec<String> = items.iter().map(render).collect();
                return format!("[{}]", inner.join(", "));
            }
            let args: Vec<String> = a.iter().map(render).collect();
            format!("({c} {})", args.join(" "))
        }
        Tree::Int(n) => n.to_string(),
        Tree::Float(x) => format!("{x:?}"),
        Tree::Str(x) => format!("{x:?}"),
        Tree::Var(v) => v.clone(),
        Tree::Other(o) => o.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_round_trip() {
        for v in [
            Value::iri("urn:ex:a"),
            Value::str("hi"),
            Value::Int(-7),
            Value::Double(1.5),
            Value::Bool(true),
            Value::literal("hi", None, Some("en")),
            Value::literal("2024-01-02", Some(tm_core::vocab::XSD_DATE), None),
            Value::Node(3),
            Value::BNode(4),
            Value::Tx(TxId(9)),
            Value::Stmt(Eid::new(12)),
        ] {
            assert_eq!(to_value(&from_value(&v)), Some(v.clone()), "{v:?}");
        }
    }

    #[test]
    fn lists_decode() {
        let l = app(
            "::",
            vec![
                eid(Eid::new(1)),
                app("::", vec![eid(Eid::new(2)), Tree::Const("[]".into())]),
            ],
        );
        assert_eq!(list(&l).map(|v| v.len()), Some(2));
        assert_eq!(render(&l), "[(eid 1), (eid 2)]");
    }
}
