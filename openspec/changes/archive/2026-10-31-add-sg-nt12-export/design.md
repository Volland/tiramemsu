## Context

LeanRDF's `leanrdf-statements` defines the format and proves it exact. tiramemsu already renders a statement as `urn:tiramemsu:stmt:N`, and its `now`, `as_of`, `history` and `valid_at` views mean what LeanRDF's views mean.

## Decisions

### LeanRDF's format, not a tiramemsu variant
The export writes exactly what `leanrdf-sg export` writes for the same records, so LeanRDF is the checker.
- Rejected: a tiramemsu-specific vocabulary, which nothing proves.
- Rejected: N-Quads with named graphs, which can't carry both clocks.

### Liveness from the engine's views
The base triple is asserted when the statement is in the selected view's `triples`, so the engine's own time semantics decide. The cross-check confirms that they agree with LeanRDF's `Record.live`.

### Valid time as `tm:validFromMs` / `tm:validToMs`
The virtual predicates `tm:validFrom` and `tm:validTo` are already `xsd:dateTime`. LeanRDF exports epoch milliseconds, because its decoder is proven on integers. Giving the export its own names avoids an IRI with two datatypes. The user chose this on 2026-10-31, over keeping `tm:validFrom` as an integer and over proving a calendar codec in Lean.

### Order and escaping copied from LeanRDF
The writer is local to `sg.rs`. It mirrors LeanRDF's derived `Ord` on `Term` (`iri < bnode < lit < triple`, then fields) and its canonical string escapes. The shared `write_term` is not changed: SPARQL results use it, and it escapes BACKSPACE as `\u0008`.

### Left out
`sys:` statements are bookkeeping, as in the plain export. A statement added and retracted in one transaction has `t_add = t_ret`: no view ever shows it, and LeanRDF's `WellFormed` requires `txAdded < txRetracted`.

## Constraints

- LeanRDF's `statement-graphs` spec (v0.3.1) binds the format.
- Canonical RDF 1.2 N-Triples (§3) binds the output.
