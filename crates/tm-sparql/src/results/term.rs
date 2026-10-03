//! ObjectId values → RDF terms (design D9).
//!
//! The terms and the renderer live in [`tm_core::rdf`], so that serialising a
//! value (the facade's bundle N-Triples, for one) needs no SPARQL parser; this
//! module re-exports them under their historical path.

pub use tm_core::rdf::{datetime_lexical, render, RdfTerm, RdfTriple};
