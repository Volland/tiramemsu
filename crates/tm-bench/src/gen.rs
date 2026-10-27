//! Seeded data generators: a WatDiv-style e-commerce graph and the cyclic graphs
//! of the triangle benchmark. Every engine receives the same triples.

/// The namespace of generated IRIs.
pub const NS: &str = "urn:wd:";

/// The full IRI of a generated predicate: `type` is `rdf:type`, so Cypher labels
/// and SPARQL `a` see it.
pub fn predicate_iri(p: &str) -> String {
    if p == "type" {
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type".to_string()
    } else {
        format!("{NS}{p}")
    }
}

/// An object: an IRI (local name in [`NS`]), an integer or a plain string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum O {
    Iri(String),
    Int(i64),
    Str(String),
}

/// One generated triple; subject and predicate are local names in [`NS`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Triple {
    pub s: String,
    pub p: String,
    pub o: O,
}

/// A small deterministic generator (no dependency), as in `examples/triangles.rs`.
#[derive(Clone, Debug)]
pub struct Lcg(pub u64);

impl Lcg {
    /// Next value in `0..n`.
    pub fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % n.max(1)
    }

    /// A skewed value in `0..n`: small values are much more likely (Zipf-like).
    pub fn skewed(&mut self, n: u64) -> u64 {
        let a = self.below(n.max(1));
        let b = self.below(a + 1);
        self.below(b + 1)
    }
}

fn t(s: String, p: &str, o: O) -> Triple {
    Triple {
        s,
        p: p.to_string(),
        o,
    }
}

fn iri(x: String) -> O {
    O::Iri(x)
}

/// The sizes of a WatDiv-style dataset of about `triples` statements.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Shape {
    pub users: u64,
    pub products: u64,
    pub retailers: u64,
    pub reviews: u64,
    pub cities: u64,
    pub genres: u64,
}

impl Shape {
    /// About half of the statements describe users, 15 % products, a third reviews
    /// (after repeated skewed edges are dropped).
    pub fn for_triples(triples: u64) -> Shape {
        let users = (triples / 19).max(20);
        Shape {
            users,
            products: (users / 2).max(10),
            retailers: (users / 50).max(3),
            reviews: users * 2,
            cities: (users / 100).max(5),
            genres: 20,
        }
    }
}

/// A WatDiv-style graph: users who follow and like, products with genres and
/// retailers, reviews with ratings, retailers and users located in cities.
/// Follows, likes and review targets are skewed, as WatDiv's degree
/// distributions are.
pub fn watdiv(triples: u64, seed: u64) -> Vec<Triple> {
    let sh = Shape::for_triples(triples);
    let mut r = Lcg(seed);
    let mut out = Vec::with_capacity(triples as usize + 1024);
    for c in 0..sh.cities {
        out.push(t(format!("city{c}"), "name", O::Str(format!("City {c}"))));
        out.push(t(
            format!("city{c}"),
            "inCountry",
            iri(format!("country{}", c % 7)),
        ));
    }
    for g in 0..sh.genres {
        out.push(t(format!("genre{g}"), "name", O::Str(format!("Genre {g}"))));
    }
    for k in 0..sh.retailers {
        let s = format!("retailer{k}");
        out.push(t(s.clone(), "type", iri("Retailer".into())));
        out.push(t(s.clone(), "name", O::Str(format!("Retailer {k}"))));
        out.push(t(
            s,
            "locatedIn",
            iri(format!("city{}", r.below(sh.cities))),
        ));
    }
    for p in 0..sh.products {
        let s = format!("product{p}");
        out.push(t(s.clone(), "type", iri("Product".into())));
        out.push(t(s.clone(), "title", O::Str(format!("Product {p}"))));
        out.push(t(s.clone(), "price", O::Int(1 + r.below(200) as i64)));
        out.push(t(
            s.clone(),
            "hasGenre",
            iri(format!("genre{}", r.skewed(sh.genres))),
        ));
        for _ in 0..2 {
            out.push(t(
                s.clone(),
                "soldBy",
                iri(format!("retailer{}", r.below(sh.retailers))),
            ));
        }
    }
    for u in 0..sh.users {
        let s = format!("user{u}");
        out.push(t(s.clone(), "type", iri("User".into())));
        out.push(t(s.clone(), "name", O::Str(format!("User {u}"))));
        out.push(t(s.clone(), "age", O::Int(18 + r.below(60) as i64)));
        out.push(t(
            s.clone(),
            "livesIn",
            iri(format!("city{}", r.skewed(sh.cities))),
        ));
        for _ in 0..3 {
            out.push(t(
                s.clone(),
                "follows",
                iri(format!("user{}", r.skewed(sh.users))),
            ));
        }
        for _ in 0..3 {
            out.push(t(
                s.clone(),
                "likes",
                iri(format!("product{}", r.skewed(sh.products))),
            ));
        }
    }
    for v in 0..sh.reviews {
        let s = format!("review{v}");
        out.push(t(
            s.clone(),
            "about",
            iri(format!("product{}", r.skewed(sh.products))),
        ));
        out.push(t(
            s.clone(),
            "reviewer",
            iri(format!("user{}", r.below(sh.users))),
        ));
        out.push(t(s, "rating", O::Int(1 + r.below(5) as i64)));
    }
    dedup(out)
}

/// Removes repeated triples (a skewed draw can repeat an edge), keeping order.
fn dedup(v: Vec<Triple>) -> Vec<Triple> {
    let mut seen = std::collections::HashSet::with_capacity(v.len());
    v.into_iter().filter(|t| seen.insert(t.clone())).collect()
}

/// `n` nodes with out-degree 5 (`k` edges), as the triangle benchmark's uniform graph.
pub fn uniform(n: u64) -> Vec<Triple> {
    (1..=n)
        .flat_map(|i| (1..=5).map(move |k| (i, (i * 7919 + k) % n + 1)))
        .map(|(a, b)| t(format!("n{a}"), "k", iri(format!("n{b}"))))
        .collect::<Vec<_>>()
        .pipe(dedup)
}

/// `hubs × spokes` edges both ways plus `noise` random edges, as the triangle
/// benchmark's hub-and-spoke graph: binary joins blow up on it.
pub fn skewed(hubs: u64, spokes: u64, noise: u64, seed: u64) -> Vec<Triple> {
    let mut r = Lcg(seed);
    let mut e = Vec::new();
    for h in 1..=hubs {
        for s in 0..spokes {
            let s = 1_000_000 + s;
            e.push((h, s));
            e.push((s, h));
        }
    }
    let span = 4 * spokes.max(1);
    for _ in 0..noise {
        e.push((2_000_000 + r.below(span), 2_000_000 + r.below(span)));
    }
    dedup(
        e.into_iter()
            .map(|(a, b)| t(format!("n{a}"), "k", iri(format!("n{b}"))))
            .collect(),
    )
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}

impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generators_are_seeded_and_sized() {
        let a = watdiv(10_000, 7);
        assert_eq!(a, watdiv(10_000, 7));
        assert_ne!(a, watdiv(10_000, 8));
        assert!((8_000..14_000).contains(&a.len()), "{}", a.len());
        assert!(!skewed(3, 50, 100, 1).is_empty());
        assert_eq!(uniform(100).len(), 500);
    }
}
