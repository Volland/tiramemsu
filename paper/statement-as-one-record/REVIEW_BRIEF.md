# Review brief: The Statement as One Record (revision 1)

You are reviewing a preprint draft as a critical referee for a database or semantic-web venue. Do not edit the manuscript. Write your report to `paper/statement-as-one-record/REVIEW.md`.

## Read these

1. `statement-as-one-record.pdf` (11 pages), or the source `statement-as-one-record.tex`
2. `SOURCE_AUDIT.md`: what was checked against sources, the corrections already made, and what is still unread
3. `references.bib`
4. For engine claims, the `lat.md/` sections listed in `SOURCE_AUDIT.md`. Use `lat section "<id>"` to read one.

The primary external source is [Gelling, Fletcher and Schmidt, arXiv 2304.13097](https://arxiv.org/abs/2304.13097) (Definitions 4.1 and 5.4 and the image definitions). The second is [Schmidt et al., ISWC 2024, CEUR Vol. 3828 paper 29](https://ceur-ws.org/Vol-3828/paper29.pdf).

## What the paper claims

- **Thesis:** OneGraph and Statement Graphs make the statement the unit beneath RDF, RDF-star and LPG, but leave storage open. Storing a statement as one record `(e, s, p, o)` with two clocks turns some statement-graph axioms into types and adds constraints the logical model lacks.
- **Definition 4 and Proposition 1:** a translation `SG` from a snapshot to a statement graph. Under endpoint closure, `SG` meets every condition of Definition 4.1 except acyclicity.
- **Corollary 2:** with added default-graph memberships and skolem IRIs, every snapshot is in the OneGraph image. It is in the RDF-star image iff no two records share `(s, p, o)`. It forms an LPG image iff every record with an eid endpoint is a property of a relationship.
- **Constraints C1–C10**, summarised in Table 1.
- **Theorem 3:** under C3, C4 and C5, every transaction-time snapshot is closed under reference.
- **Proposition 4:** if every insert references only earlier rows, reference is well-founded, so `SG(D@t)` is acyclic.
- **Section 6:** benefits. **Section 7:** costs, including Table 2 (reused benchmark) and the places where Statement Graphs are stronger.

## Questions to answer

1. **Proofs.** Is Theorem 3's induction sound? Check in particular a record inserted and its target retracted later in the same transaction, and the `a ≤ d` lifetime. Does Proposition 4's premise really follow from C3 for every write path the paper names (assert, create, confirm, supersede, membership)? Is Corollary 2's LPG condition exactly right against the LPG image (label statements, edges between nodes, properties of nodes and edges)?
2. **Fidelity to the sources.** Does the paper describe Statement Graphs, the OneGraph image (defined per connected component) and Neptune Analytics accurately and fairly? Flag any claim about Neptune that its papers do not support.
3. **Contribution.** Is there enough here beyond the 6 October site article (`site/articles/statement-graphs.html`) and the two companion papers? Are Theorem 3 and Proposition 4 near-definitional? If so, say what result would make the paper worth submitting.
4. **The constraint list.** Is C1–C10 complete, non-overlapping and correctly placed in Table 1? Is any constraint stated more strongly than the engine enforces? The engine does not enforce acyclicity for raw-SQL writes.
5. **Costs.** Is Table 2 used honestly? It is reproduced from an earlier article, not re-run, and compares against quads with reifiers, not Neptune.
6. **Writing.** Flag unclear passages, undefined notation, inconsistent terms (statement, record, row, occurrence) and anything that reads as marketing.

## Out of scope

- The Rust code and `lat check` errors. A parallel agent is working on vector recall; ignore those errors.
- Performance experiments. The paper reports none of its own.

## Report format

Use the format of `paper/metagraphs-from-identified-triples/REVISION_NOTES.md` so the response can map to it:

1. A summary verdict: accept as is, minor revision, major revision, or not a paper yet. Give one paragraph of reasons.
2. A numbered list of points. Each gives its severity (blocking, major, minor), its location (section, definition or line), the problem, and a concrete fix.
3. Any source checks you did yourself, with links.
