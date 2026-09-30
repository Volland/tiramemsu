//! Expected values of the TCK (Neo4j result notation) and their comparison.
//!
//! Adapted from oxilite's `tests/tck.rs` (MIT OR Apache-2.0).
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use tm_cypher::CypherValue;

// ----- expected values (Cypher literal syntax of the TCK) -----

#[derive(Debug, Clone, PartialEq)]
pub enum TV {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<TV>),
    Map(BTreeMap<String, TV>),
    Node(BTreeSet<String>, BTreeMap<String, TV>),
    Rel(String, BTreeMap<String, TV>),
    /// Nodes and relationships with their direction (`true` = forward).
    Path(Vec<TV>, Vec<(TV, bool)>),
}

struct VP<'a> {
    s: &'a [u8],
    i: usize,
}

impl VP<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn peek(&mut self) -> u8 {
        self.ws();
        self.s.get(self.i).copied().unwrap_or(0)
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == c {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn name(&mut self) -> String {
        self.ws();
        if self.s.get(self.i) == Some(&b'`') {
            self.i += 1;
            let st = self.i;
            while self.i < self.s.len() && self.s[self.i] != b'`' {
                self.i += 1;
            }
            let n = String::from_utf8_lossy(&self.s[st..self.i]).to_string();
            self.i += 1;
            return n;
        }
        let st = self.i;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_')
        {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[st..self.i]).to_string()
    }

    fn map(&mut self) -> Option<BTreeMap<String, TV>> {
        let mut m = BTreeMap::new();
        if !self.eat(b'{') {
            return Some(m);
        }
        if self.eat(b'}') {
            return Some(m);
        }
        loop {
            let k = self.name();
            if !self.eat(b':') {
                return None;
            }
            let v = self.value()?;
            m.insert(k, v);
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Some(m);
            }
            return None;
        }
    }

    fn node(&mut self) -> Option<TV> {
        if !self.eat(b'(') {
            return None;
        }
        let mut labels = BTreeSet::new();
        while self.eat(b':') {
            labels.insert(self.name());
        }
        let props = if self.peek() == b'{' {
            self.map()?
        } else {
            BTreeMap::new()
        };
        if !self.eat(b')') {
            return None;
        }
        Some(TV::Node(labels, props))
    }

    fn rel(&mut self) -> Option<TV> {
        if !self.eat(b'[') {
            return None;
        }
        if !self.eat(b':') {
            return None;
        }
        let t = self.name();
        let props = if self.peek() == b'{' {
            self.map()?
        } else {
            BTreeMap::new()
        };
        if !self.eat(b']') {
            return None;
        }
        Some(TV::Rel(t, props))
    }

    fn value(&mut self) -> Option<TV> {
        match self.peek() {
            b'(' => self.node(),
            b'<' => {
                self.i += 1;
                let mut nodes = vec![self.node()?];
                let mut rels = Vec::new();
                loop {
                    match self.peek() {
                        b'>' => {
                            self.i += 1;
                            return Some(TV::Path(nodes, rels));
                        }
                        b'<' => {
                            self.i += 1;
                            self.eat(b'-');
                            let r = self.rel()?;
                            self.eat(b'-');
                            rels.push((r, false));
                            nodes.push(self.node()?);
                        }
                        b'-' => {
                            self.i += 1;
                            let r = self.rel()?;
                            self.eat(b'-');
                            self.eat(b'>');
                            rels.push((r, true));
                            nodes.push(self.node()?);
                        }
                        _ => return None,
                    }
                }
            }
            b'[' => {
                // A relationship `[:T]` or a list.
                let save = self.i;
                self.i += 1;
                if self.peek() == b':' {
                    self.i = save;
                    return self.rel();
                }
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Some(TV::List(items));
                }
                loop {
                    items.push(self.value()?);
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b']') {
                        return Some(TV::List(items));
                    }
                    return None;
                }
            }
            b'{' => self.map().map(TV::Map),
            b'\'' => {
                self.i += 1;
                let mut out = Vec::new();
                while self.i < self.s.len() && self.s[self.i] != b'\'' {
                    if self.s[self.i] == b'\\' && self.i + 1 < self.s.len() {
                        self.i += 1;
                        match self.s[self.i] {
                            b'n' => out.push(b'\n'),
                            b't' => out.push(b'\t'),
                            c => out.push(c),
                        }
                    } else {
                        out.push(self.s[self.i]);
                    }
                    self.i += 1;
                }
                self.i += 1;
                Some(TV::Str(String::from_utf8_lossy(&out).to_string()))
            }
            _ => {
                let st = self.i;
                while self.i < self.s.len() && !b",]}) \t".contains(&self.s[self.i]) {
                    self.i += 1;
                }
                let tok = std::str::from_utf8(&self.s[st..self.i]).ok()?.to_string();
                match tok.as_str() {
                    "null" => Some(TV::Null),
                    "true" => Some(TV::Bool(true)),
                    "false" => Some(TV::Bool(false)),
                    "NaN" => Some(TV::Float(f64::NAN)),
                    "Infinity" | "+Infinity" => Some(TV::Float(f64::INFINITY)),
                    "-Infinity" => Some(TV::Float(f64::NEG_INFINITY)),
                    t => {
                        if let Ok(i) = t.parse::<i64>() {
                            Some(TV::Int(i))
                        } else if let Some(h) = t.strip_prefix("0x") {
                            i64::from_str_radix(h, 16).ok().map(TV::Int)
                        } else if let Ok(f) = t.parse::<f64>() {
                            Some(TV::Float(f))
                        } else if t.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                            // an unquoted temporal literal: compared as text
                            Some(TV::Str(t.to_string()))
                        } else {
                            None
                        }
                    }
                }
            }
        }
    }
}

pub fn parse_tv(s: &str) -> Option<TV> {
    let mut p = VP {
        s: s.as_bytes(),
        i: 0,
    };
    let v = p.value()?;
    p.ws();
    (p.i == s.len()).then_some(v)
}

/// Turns one of our result values into the TCK notation.
pub fn to_tv(v: &CypherValue) -> TV {
    let props =
        |p: &BTreeMap<String, CypherValue>| p.iter().map(|(k, v)| (k.clone(), to_tv(v))).collect();
    match v {
        CypherValue::Null => TV::Null,
        CypherValue::Boolean(b) => TV::Bool(*b),
        CypherValue::Integer(i) => TV::Int(*i),
        CypherValue::Float(f) => TV::Float(*f),
        CypherValue::String(s) => TV::Str(s.clone()),
        CypherValue::Date(_) | CypherValue::DateTime { .. } | CypherValue::LocalDateTime(_) => {
            match v.to_json() {
                serde_json::Value::String(s) => TV::Str(s),
                other => TV::Str(other.to_string()),
            }
        }
        CypherValue::List(l) => TV::List(l.iter().map(to_tv).collect()),
        CypherValue::Map(m) => TV::Map(props(m)),
        CypherValue::Node(n) => TV::Node(n.labels.iter().cloned().collect(), props(&n.properties)),
        CypherValue::Relationship(r) => TV::Rel(r.rel_type.clone(), props(&r.properties)),
        CypherValue::Path(p) => {
            let nodes = p
                .nodes
                .iter()
                .map(|n| to_tv(&CypherValue::Node(Box::new(n.clone()))))
                .collect();
            let mut rels = Vec::new();
            for (i, r) in p.rels.iter().enumerate() {
                let forward = p
                    .nodes
                    .get(i)
                    .is_some_and(|n| n.element_id == r.start_element_id);
                rels.push((
                    to_tv(&CypherValue::Relationship(Box::new(r.clone()))),
                    forward,
                ));
            }
            TV::Path(nodes, rels)
        }
    }
}

pub fn tv_eq(a: &TV, b: &TV, unordered_lists: bool) -> bool {
    match (a, b) {
        (TV::Float(x), TV::Float(y)) => {
            (x.is_nan() && y.is_nan()) || x == y || ((x - y).abs() <= 1e-9 * x.abs().max(y.abs()))
        }
        (TV::List(x), TV::List(y)) => {
            if x.len() != y.len() {
                return false;
            }
            if unordered_lists {
                multiset_eq(x, y, true)
            } else {
                x.iter().zip(y).all(|(p, q)| tv_eq(p, q, false))
            }
        }
        (TV::Map(x), TV::Map(y))
        | (TV::Node(_, x), TV::Node(_, y))
        | (TV::Rel(_, x), TV::Rel(_, y)) => {
            let labels_ok = match (a, b) {
                (TV::Node(l1, _), TV::Node(l2, _)) => l1 == l2,
                (TV::Rel(t1, _), TV::Rel(t2, _)) => t1 == t2,
                _ => true,
            };
            labels_ok
                && x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| tv_eq(v, w, unordered_lists)))
        }
        (TV::Path(n1, r1), TV::Path(n2, r2)) => {
            n1.len() == n2.len()
                && r1.len() == r2.len()
                && n1.iter().zip(n2).all(|(p, q)| tv_eq(p, q, false))
                && r1
                    .iter()
                    .zip(r2)
                    .all(|((p, d1), (q, d2))| d1 == d2 && tv_eq(p, q, false))
        }
        _ => a == b,
    }
}

pub fn multiset_eq(x: &[TV], y: &[TV], unordered_lists: bool) -> bool {
    let mut used = vec![false; y.len()];
    'outer: for a in x {
        for (j, b) in y.iter().enumerate() {
            if !used[j] && tv_eq(a, b, unordered_lists) {
                used[j] = true;
                continue 'outer;
            }
        }
        return false;
    }
    true
}

pub fn rows_eq(
    actual: &[Vec<TV>],
    expected: &[Vec<TV>],
    ordered: bool,
    unordered_lists: bool,
) -> bool {
    if actual.len() != expected.len() {
        return false;
    }
    let row_eq = |a: &Vec<TV>, b: &Vec<TV>| {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| tv_eq(x, y, unordered_lists))
    };
    if ordered {
        return actual.iter().zip(expected).all(|(a, b)| row_eq(a, b));
    }
    let mut used = vec![false; expected.len()];
    'outer: for a in actual {
        for (j, b) in expected.iter().enumerate() {
            if !used[j] && row_eq(a, b) {
                used[j] = true;
                continue 'outer;
            }
        }
        return false;
    }
    true
}
