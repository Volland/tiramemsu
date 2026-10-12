# Source audit — revision 4, 12 October 2026

## Revision 4 additions

- Gelling et al. arXiv 2304.13097v1 Definition 5.3 (LPG image) re-read for §9.2: subject targets in nodes ∪ statements, predicate targets in keys ∪ label sets, object targets in values ∪ nodes ∪ label sets; edge, node-label and property statements, the latter two without incoming edges; edge properties as property statements about edge statements (Appendix 10.1). The proposition uses only these typing and statement-kind conditions.
- RDF 1.2 terminology (triple terms in object position only, reifiers, `rdf:reifies`, assertion as a separate triple) follows the dated CR snapshot already cited; no claim about later standard status.
- The engine's acceptance of correction cycles and rejection of direct self-reference: `lat.md/tests.md`, "Correction Remaps Patched References" and "Invalid Correction Endpoints Roll Back". The acyclic-replay rule is modelled in the validator only.
- Appendix B (inherited timings) removed; the blog article is no longer cited.

## Revision 3 additions

- Gelling et al. arXiv 2304.13097v1, Definitions 4.1, 5.1 and 5.4, re-read: non-membership statement vertices need at least one incoming edge; membership statements have a statement subject, IRI or blank-node object and no incoming edges; the OneGraph image disregards vertex isomorphism. The temporal encoding relies on the incoming-edge condition for unasserted records.
- Engine behavior for Examples 1 and 2 was executed, not only read: `crates/tm-core/tests/export_boundary.rs` on the working tree (version string 0.4.0, after commit `5a55efc`). `engine-trace.json` holds the dumped rows; its hash is in `validation-results.json`.
- New references (Datomic, XTDB, SQL:2011, temporal RDF, Wikidata, named graphs, Kahn, Jones–Lins) are cited for well-known, bounded descriptions; the two product documentation sites were not re-audited page by page.

# Source audit — revision 2

This file records the evidence used in the review and revision. It supersedes revision 1's audit, whose claims about universal closure and cycle rejection were incorrect.

## Primary external sources

| Source | Evidence and scope |
| --- | --- |
| [Gelling, Fletcher and Schmidt, arXiv 2304.13097v1](https://arxiv.org/html/2304.13097v1) | Raw HTML Definitions 4.1 and 5.1–5.4 and mapping/prototype sections were read during review. Technical image claims use this version. The revision explicitly constructs typed leaves, reserved membership forms and injective labels. |
| [DASFAA 2024 publication](https://doi.org/10.1007/978-981-97-5575-2_27) | Publication metadata only; separate full text not obtained. It is not silently substituted for the inspected preprint. |
| [Lassila et al., OneGraph vision, journal-hosted PDF](https://www.semantic-web-journal.net/system/files/swj3273.pdf) | Relevant text inspected during review, including §3.7 on update semantics and the identity discussion. The PDF has prepublication page furniture; final-version textual equivalence was not audited. It explicitly motivates the duplicate-occurrence deletion problem. |
| [Schmidt et al., openCypher over RDF, ISWC 2024](https://ceur-ws.org/Vol-3828/paper29.pdf) | PDF inspected. Supports OneGraph concepts in Neptune Analytics, relationship/property statements, identity coexistence and update queries. Does not establish exact identity with the formal Statement Graph image or a physical storage layout. |
| [RDF 1.2 Concepts, dated 7 April 2026 snapshot](https://www.w3.org/TR/2026/CR-rdf12-concepts-20260407/) | Relevant triple-term/reifier and dataset sections inspected. Unasserted propositions are explicitly outside the revision's occurrence-export contract. The dated snapshot is cited, not an inferred current standards status. |
| Hartig 2017; Nguyen et al. 2014; Hernández et al. 2015; Angles et al. 2022; Vrgoč et al. 2023 | Inherited bibliographic references used for bounded related-work descriptions; not newly audited in full for this revision. |

## Engine version and decisive checks

Engine claims are pinned to **v0.4.0**, commit `6e7961c0378effc1793896395c464c8af460566b`. The working repository contains unrelated later changes; those were not used to redefine the release's guarantees.

| Claim | Evidence |
| --- | --- |
| Ordinary endpoint liveness has a historical-lineage exception | Tag's `lat.md/time-model.md`, Assert and Supersede sections; `sys:supersedes` may target an allocated historical occurrence. |
| Atomic correction can produce a cycle | Tag's `lat.md/tests.md`, “Correction Remaps Patched References”; the resulting cycle is complete and live. “Invalid Correction Endpoints Roll Back” rejects direct self-reference and invalid/dropped endpoints, not all cycles. |
| Import rejects cyclic bundles | Tag's `lat.md/tests.md`, “Import Rejects Cycles And Malformed Bundles”; `lat.md/data-model.md`, Fact Bundles. |
| Memberships may be annotated and graph names may be eids | `lat.md/data-model.md`, Named Graphs and Statement Graphs. The export treats these as ordinary structural content, not native target memberships. |
| Immutability, retraction and time | `lat.md/storage.md`, Invariant Triggers; `lat.md/time-model.md`, lifecycle and operations. File-level guards assume intact schema/metadata. |
| Prior temporal theory already handles the exceptions | `paper/layered-bitemporal-graphs.tex`, structural/lineage distinction and conditional acyclicity proposition. |

These are documentation/source checks, not newly executed engine traces. No Rust code was edited.

## Inherited measurements and new validation

The timing table is reproduced from `site/articles/tiramemsu-vs-oxilite.html`, titled *How tiramemsu differs from oxilite*, dated 30 September 2026. The script `bench/eid-vs-reifier/bench.py` was read but not rerun. Exact generated counts, absent temporal/graph columns, differing layouts and writes, `synchronous=OFF`, fixed timing order and limited equality checking are disclosed. No hardware specification, variance estimate or new benchmark result was invented.

`validate.py` is new mathematical validation specific to revision 2. `validation-results.json` gives actual run counts and the script hash. It checks finite candidate stores against an independent subset oracle, not the engine or the whole RDF/LPG standards. The LaTeX includes its generated count paragraph.

The previous site exposition and both companion papers are explicitly credited. No first-in-literature claim is made for the row representation, temporal closure argument, SCC algorithm, or order-based acyclicity argument.
