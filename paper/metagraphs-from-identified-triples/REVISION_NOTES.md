# Revision notes

## Revision 2 — 5 October 2026 (response to review)

The review's central criticism was that most theorems are definitional, so the paper now says so and promotes the constraint set as the contribution. Each point and what was done:

| Review point | Action |
| --- | --- |
| 1. Theorems 3–5, 6, 8, 11 are near-definitional | Roundtrips are now propositions reduced by one lemma (clause bijection) to comparing source restrictions with image constraints. The introduction states the thesis (identity is cheap, invertibility is a separate obligation) and the paper adds a necessity result: for each of 18 image constraints a witness violating only that constraint, checked mechanically (Table 2). The rank ceiling and compact depth are kept but described as properties of the anchor choice and the rank recurrence. |
| 2. Rank has no consumer | Added bulk-load strata (minimum number of referentially complete batches is `1 + max rank`) and retraction-cascade depth (rounds at most `max rank − rank(e)`), both proved and tested on random stores. These are about passes over the reference graph. Join-depth bounds and stratified query evaluation were not attempted. |
| 3. "Normal forms" misnomer | Renamed to row fragments (`Row_1..Row_4`) and removed from the title. No normalization result was added. The structural reduct is described as a projection (deletion), not a normalization. |
| 4. Valid-time containment wrong for annotations | The snapshot theorem now has two policies: containment (as before) and retained referents, where an asserted annotation keeps its referents filtered by transaction time only. The statement covers both, shows containment is the special case, and gives the "disputed in 2025" example. |
| 5. Identity across versions | Added a supersession relation (lineage rows) with an admissibility condition and a proposition that chains give stable identity. Propagating a revision to dependents is stated as out of scope (it is the cascade-and-replay of the first paper). |
| 6. Extensional images fragile under time | Added a remark with the two-edges-different-valid-times example, checked by the checker; it favours the identified images. |
| 7. SPARQL and the carrier | Stated the RDF 1.2 reifier mapping explicitly; the path binds the reifier resource, not the anchor node; plain RDF 1.1 and RDF-star do not provide it; the engine's identifier-as-node is an extension. The path expression in the proposition and in the text is now one macro. |
| 8. Small validation; 2,411 vs 2,401 | The relation is stated and asserted in the checker: 2,411 = 1 + 9 + 2,401 higher-order sources, and the 2,401 accepted candidate stores are exactly the two-edge sources. Added seeded randomized tests (up to 8 vertices and 8 edges), 620,992 exhaustive firing-closure cases, and mutation testing. Temporal checks are still sampled, and the sequence oracle's randomized domain is small because it is exponential. |
| 9. Proof rigor | Isomorphism is now defined (Definition 2) with a model-side counterpart; the clause-bijection lemma makes the "as in Theorem 3" proofs explicit. A Lean 4 proof covers the directed, higher-order and extensional roundtrip under canonical naming only. Recursive, attributed and temporal results are not mechanized. |

Smaller issues: contribution list has a lead-in; "storage-size bounds" is now "row counts"; the redundant `e ∉ {s_e, o_e}` was removed (acyclicity subsumes it) and the `P ⊆ N` restriction is noted against Meta-Property Graphs; the NF3 container requirement is now explicit and the F2 ⊂ F3 witness changed to a MetaVertex anchor; the loop-free restriction on membership graphs was removed; the abstract keeps two disclaimers and the rest moved to Section 11.

Checked against sources: the Basu–Blanning edge definition as reproduced by Parsonage et al. (Definition 2.2) allows empty invertex or outvertex and imposes no disjointness, so `J_BB` needs no `V ∩ W = ∅` constraint (the paper says so). The simple-path definition (Definition 2.5 there) is the repetition-permitting one used here. The Basu–Blanning book text itself was not accessed, so it is cited through that reproduction as well as directly.

Added related work: the W3C n-ary relation note (checked, 12 April 2006), Hernández et al. (reused from the first paper's bibliography), RDF 1.2 reifiers, TypeDB role-playing relations (documentation checked), hyper-relational knowledge graphs (Galkin et al., Crossref-checked), the OpenCog framework chapter (Crossref-checked metadata only; the links-to-links statement is a description of the pattern, not a full-text citation claim), the Poulovassilis–Levene nested-graph model (Crossref-checked; a first DOI recalled from memory was wrong and was replaced), and sequenced temporal referential integrity (Snodgrass; Kulkarni and Michels).

New result: executable metapaths (firing closure). Recognition is linear; the closure lies inside walk reachability and equals it for singleton tails; a walk metapath is executable whenever the dependency graph is acyclic or tails are singletons; Example 1 is the cyclic counterexample. All four statements are tested (51,176 antecedent cases and 50,143 strict-gap cases).

Not done: SHACL/SPARQL validators are tested for agreement with the Python validators on the stated sets (see `validators/README.md`); the engine still does not enforce the images. Restructuring was partial: the thesis now leads and rank moved after temporal integrity and paths, but the section order is otherwise as before. The review's scores are not claimed to have changed.

Validators: `validators/` holds SHACL core shapes and six SPARQL queries over a projected row view (RDF 1.2 is not available in the toolchain). The agreement test compares them with the Python validators on 80,665 stores (D 3,702; BB 3,702; HO 69,151; U 305; A 3,805) with zero disagreements, exhaustive for the roundtrip sources, the 65,536 role-subset stores and the member subsets, sampled for attributed stores and mutations. It was re-run independently after the delegating run (3 minutes on 12 cores). Lean: `lean/` builds offline with `lake build` and contains no `sorry`, axiom or `native_decide`.

## Revision 1 — 5 October 2026

The paper was rewritten as a theoretical representation paper. Obsolete drafts and planning files were removed from this folder during the final cleanup requested by the author.

### Scientific changes

1. Separated broad feature languages from exact encoding images. Directed, higher-order, recursive-member, and four-class attributed images now state all permitted rows, endpoint types, cardinalities, and uniqueness constraints.
2. Defined identified higher-order source edges independently of their endpoint sets. Explicitly prohibited self-end incidence while retaining cycles between different edges. Kept parallel identified edges separate from the extensional Basu–Blanning domain.
3. Added the recursive-member roundtrip theorem with well-foundedness and extensionality. Limited the ubergraph correspondence to the nonempty, atom-based fragment; added an explicit longest-member-path depth proof.
4. Distinguished all four attributed atom classes, including empty-fragment metaedges. Direction flags and unique src/tgt roles are obligatory fields; ordinary attributes have an explicit ground-value set semantics.
5. Replaced rank/reflection equivalence with reference-dependency depth. Added counterexamples with confidence at ranks one and two, and a figure separating decoded member incidence from statement references.
6. Added exact row-count formulas and qualified linear encoding/decoding costs by the identity and set-operation assumptions.
7. Chose candidate-restricted, repetition-permitting metapath semantics; gave an incidence-graph recognition procedure and complexity proof. Added the cyclic-input counterexample separating metapath recognition from operational AND-enablement.
8. Restricted projection claims to explicitly transferred external operators and distinguished a structural reduct from enriched provenance. Clarified that alternative supports require maintenance logic.
9. Proved snapshot commutation under whole mandatory-component selection and dependency interval containment. Identified independently timed role withdrawal and read-time effective-validity intersection as outside that theorem's contract.
10. Rewrote memory as an illustrative provenance example with explicit status assertions. Removed cognitive-fidelity and retrieval-quality implications and reduced repeated dependence on the author's informal book.
11. Added a standard-library bounded checker with independently scanned image decoding and an independent edge-sequence metapath oracle. Retained bounds and actual results in JSON and generated the paper's count table from those results.

### Verification performed

- The bounded checker passes. It covers 65,536 attributed roundtrips, 65,536 higher-order candidate stores (2,401 accepted, 63,135 rejected), 2,411 higher-order source roundtrips, 5,488 metapath query/oracle comparisons, 14,400 compact-depth configurations, and 20 dependency-closed temporal selections. Additional counts and bounds are in `validation-results.json`.
- `cargo test -p tiramemsu --test recipes metagraph_containers_nesting_and_fold --offline` passed: one test, zero failures.
- The revised PDF was built through pdflatex/BibTeX/pdflatex/pdflatex and rendered for visual inspection. The source-revised paper has 15 pages, no unresolved citations or cross-references, and no overfull boxes. Two harmless underfull line warnings remain in the feature-language list and availability paragraph; those pages were visually checked.

### Primary sources checked

- [Gapanyuk, 2021](https://ceur-ws.org/Vol-2965/paper01.pdf), especially the four-class tuple structure in Section 5. The revised attributed source makes identity and attribute conventions explicit rather than claiming an unspecified universal correspondence.
- [Joslyn and Nowak, 2017](https://arxiv.org/html/1704.05547v1), Definitions 1 and 4 and the uber-Levi construction. The general powerset definition admits cases outside the paper's nonempty-edge fragment; the revised scope says so.
- [Parsonage, Roughan and Nguyen, 2025](https://arxiv.org/html/2509.04543v1), Definitions 2.5–2.10. The manuscript states its path and candidate-universe choices; it does not claim an implementation of their projection algorithm.
- [Gelling, Fletcher and Schmidt preprint](https://arxiv.org/abs/2304.13097): prior statement-graph unification is acknowledged.
- [Zep](https://arxiv.org/abs/2501.13956) and [AriGraph](https://arxiv.org/abs/2407.04363): included as independent memory-system context, without transferring their outcome claims to this paper.
- [MillenniumDB publisher record](https://direct.mit.edu/dint/article/5/3/560/117375/MillenniumDB-An-Open-Source-Graph-Database-System): corrected the active bibliography DOI to `10.1162/dint_a_00229`, matching the publisher landing page. Some distributed PDFs retain the older `00209` DOI; this discrepancy is recorded rather than silently ignored. The page range remains 560–610.
- Added direct arXiv links for cited preprints so the generated bibliography retains source locators.
- The earlier paper's local `thm:lifting` and encoding definitions were inspected to avoid extending their domain without new premises.

### Focused source revision

The follow-up literature revision is documented in [SOURCE_AUDIT.md](SOURCE_AUDIT.md). Related work now explicitly credits statement-image mappings, earlier metagraph storage, and Meta-Property Graphs. Three recent metagraph publications have abstract/metadata comparisons with their full-text access limits stated. The conference precursor to the 2026 incidence/nesting article is cited separately. Confirmed page ranges, book title, series volumes, editors, issue number, and direct source links were corrected in the active bibliography.

### Remaining limits

Finite checks support the hand proofs; they are not mechanization or exhaustive verification over unbounded structures. The checker uses a bounded attribute vocabulary and fixed temporal selection examples. The three recent metagraph papers remain a full-text comparison gap despite the new abstract-level comparison; no unqualified priority claim is made. Other inherited reference metadata have not all been audited. The manuscript has no comparative performance or agent-retrieval results. These limits are stated in the paper rather than filled with invented evidence.

### Final PDF and source cleanup

Rebuilt the final PDF using `build.py`, which runs TeX and BibTeX in a temporary directory and checks unresolved references and overfull boxes before replacing the PDF. Removed obsolete drafts, the seed bibliography, the original-draft review, TeX intermediate files and a stale tool-error state. Pruned the active bibliography to the 29 cited entries. Retained the final manuscript, bibliography, checker, generated evidence, source audit and revision notes. A temporary recovery copy of removed historical files was saved outside the paper folder at `/var/folders/qn/v0qs219s5r763h90wgqxly1h0000gn/T/metagraph-paper-before-cleanup-dqrbncz4`.
