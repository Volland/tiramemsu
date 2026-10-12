# Revision notes

## Revision 4 — 12 October 2026

Responds to a meta-review (recommendation: major revision for journals, minor for short/systems tracks) with four actionable points and three weaknesses.

| Review point | Revision |
| --- | --- |
| 1. Fix or excise Appendix B | Excised. The inherited SQLite timings (`synchronous=OFF`, no variance, unequal workloads) supported no claim of the paper. Limitations now state that the paper makes no performance claims. |
| 2. RDF 1.2 reification | New §9.1: reifier encoding Ω_f and Proposition (reifier roundtrip). Reifiers remove the membership statements and the export boundary itself (references are names; the cyclic engine history roundtrips) but assert per triple, so the asserted set is recoverable exactly when no asserted record shares content with an unasserted one, which fails on Example 1 (e1′ repeats e1). The dual failure of the RDF-star encoding is stated. |
| 3. Engine enforcement of C6 | New Definition (acyclic-replay rule) and Proposition (acyclic history): a peel over the replay batch alone, O(\|C\|), rejects exactly the patches that name a member of the cascade set; then K(D)=D and every bit is set. The paper explains why tiramemsu 0.4.0 does not apply it (the cycle satisfies the engine's own invariants; export is the first DAG consumer) and recommends it for exporting stores. The engine itself was not changed. |
| 4. Query equivalence | New §8: quoting map q, Proposition (query view) — asserted graph is the q-image, q injective iff contents are distinct, BGP solutions map into RDF-star solutions and bijectively onto them when injective; extension to equality-based SPARQL operators; Corollary for snapshot answers. Duplicate contents from valid-time corrections are shown to merge answers. |
| Weakness: LPG branch | New §9.2: Proposition (LPG direct export) under Gelling et al. Def. 5.3 — only edges between node terms, unreferenced labels and unreferenced properties on nodes or edges; clocks travel for edges only; no assertion marker. Greatest LPG subset left open (not monotone). |
| Weakness: elementary proofs | Introduction reframes the contribution around engine invariants (write rule, stored bit, omission reports, time). |

Validation additions: per-store checks of the query view (four BGPs including joins through statement variables), RDF 1.2 roundtrip and asserted-set criterion on every store, LPG condition against an independent Def. 5.3 checker on every kernel; 1,500 protocol histories under the acyclic-replay rule (337 rejections, each confirmed to name a cascade member); 33 duplicate-content stores with RDF-star answers that have no store counterpart; both engine histories exported through RDF 1.2 (ambiguous asserted set exactly in the lineage trace). Not done: no new engine code, no LPG temporal theorem, no performance experiment.

## Revision 3 — 11 October 2026

Responds to the second review (framing and scope). Retitled *When Is a Temporal Statement Store a Statement Graph?* to match the content, an export boundary rather than a physical claim.

| Review point | Revision |
| --- | --- |
| 1. Export the full history | New Proposition (closed history): under immutable content, no deletion, live references and replay, the history is closed and every cycle lies inside one replay batch, so `K(D)` drops only replay cycles and dependents. New temporal encoding theorem: transaction and valid time as reserved statements; lossless roundtrip of `(S, asserted set)`. Trade-off between asserting all occurrences and asserting the snapshot while quoting history is stated, linked to RDF 1.2 unasserted propositions. |
| 2. Dual construction | New section on the reference closure `C_D(S)`: least closed superset, `K(S) ⊆ K(C(S)) ⊆ C(S)`; shown on the running example to keep the supersession provenance the kernel drops. |
| 3. Fixpoint characterization | Kernel is now defined as the least fixpoint of `F_S(X) = {r : refs(r) ⊆ X}`; greatestness by height induction; Kahn-style peel with export order; incremental exportable bit (new proposition, including closed snapshots); connection to well-founded parts and garbage collection. SCC form kept as a corollary for omission reasons. |
| 4. Real "only if" | Identity-preserving direct export is defined (conditions a, b) and the criterion is now a theorem with both directions and uniqueness; leaf substitution remark, checked by the validator. |
| 5. Cyclic corrections | States that cycles are confined to the replay batch, so a store can forbid them with a batch-local check; the paper covers both policies and says no use case is known. |
| 6. Model–engine gap | New engine test `crates/tm-core/tests/export_boundary.rs` runs both examples on both SQLite hosts and dumps raw records; the validator recomputes kernels and the temporal roundtrip from the dump. Validation extended to all four-record reference structures up to relabeling (two-cycle plus two-deep chain), random 4–7-record stores and random protocol histories with supersession. Not mechanized; kernel not an engine API. |
| Section 7 and benchmark | Benchmark moved to a short appendix with condensed caveats; C3–C6 kept in the main text, other commitments in an appendix; query-equivalence and SPARQL multiplicity remarks cut (listed as a limitation). |
| Related work | Added Datomic, XTDB, SQL:2011, Gutierrez–Hurtado–Vaisman temporal RDF, Wikidata statements, named graphs, Kahn, garbage collection. |
| Evidence hygiene | Image definitions re-checked against arXiv 2304.13097v1 Defs 4.1, 5.1, 5.4; the paper now cites that version for definitions and no longer comments on the DASFAA full text. The role of memberships is explained (incoming-edge condition, asserted vs quoted). Example dictionary `urn:tiramemsu:<db-uuid>:n/<id>` given. |
| Presentation | Abstract leads with what is exported, dropped and reported; disclaimers consolidated into one Limitations section; review residue removed from Availability; FRA/LHR/distance is the running example across sections and the figure; Table 1 lists per-operation invariants and what checks each. |

## Revision 2 — 11 October 2026

The revision replaces the universal whole-snapshot claim with a precise direct-export contract. The paper is now a representation/design study centered on the greatest closed, acyclic subset obtainable by deleting whole records without rewriting endpoints. It preserves the historical review in `REVIEW.md`; the scores there assess revision 1 and have not been relabeled as scores for revision 2.

| Review point | Revision |
| --- | --- |
| 1. Historical lineage defeats live closure | Added an explicit supersession counterexample; closure proposition is restricted to a fully stated protocol. Export checks all eid endpoints. |
| 2. Correction can create cycles | Added the documented replay-generated two-cycle. Earlier-reference insertion is conditional; supersede is excluded from that premise. |
| 3. Exact-image claims underspecified | Replaced the general corollary with an explicit RDF-star-branch encoding, injective term dictionary, reserved vocabulary, fresh memberships, and decoder. Removed the LPG iff. |
| 4. Same-transaction lifetime and induction | Uses `(previous ∪ inserted) \ retracted`; preserves `a=d` empty lifetimes and distinguishes operation liveness from committed visibility. |
| 5. Projection versus RDF/SPARQL semantics | Separates temporal erasure, occurrence collapse, RDF interpretation and query-result multiplicities. |
| 6. Historical RDF-star versus RDF 1.2 | Explicitly excludes full RDF 1.2 interchange, including unasserted proposition coverage. |
| 7. Standalone contribution and overlap | Credits both companions and the prior site article; centers on the greatest deletion-only export subset, algorithm, explicit image and validation. Describes elementary arguments as such. |
| 8. Benchmark confounding and provenance | Labels the table as reproduced, corrects logical/physical counts and the missing graph-column description, reports durability and validation limitations, and removes identifier-cost generalizations. |
| 9. Mixed constraint taxonomy | Recasts C1–C10 as design commitments, separates enforcing layers and states exceptions. |
| 10. Logical versus physical comparison | Removes any implication that Statement Graphs require four physical records or that row shape supplies temporal/cascade guarantees. |
| 11. Neptune attribution | Removes Neptune from the title and avoids equating its internal model with the entire formal image class. Adds the vision paper's update discussion. |
| 12. Examples and terminology | Defines literals and transaction terms, clarifies temporal episodes and reification counts, removes the mismatched query pair, and removes marketing-style claims. |

### New formal material

For a selected set `S`, seed invalidity with missing-reference sources and vertices on reference cycles. Remove these vertices and all records reaching them. The remaining kernel is closed and acyclic, and every other closed, acyclic subset is contained in it. The proof establishes a unique greatest subset, not merely a maximal or heuristically selected subgraph. An SCC/reverse-reachability procedure runs in linear time given indexed identifier lookup.

An explicit OneGraph structural encoding preserves eid and triple content, after an injective term conversion. Its inverse requires that dictionary. Temporal information and native graph-membership semantics are deliberately outside the roundtrip. An RDF-star image occurs exactly for duplicate-free triple content under this specified encoding.

### Validation and build

`validate.py` independently compares SCC extraction with exhaustive subset enumeration over 217,625 candidate stores. It also checks image axioms, the structural roundtrip, the duplicate criterion, idempotence, 10 focused store cases, five malformed-image mutations and the same-transaction lifetime boundary. No disagreements were observed. Results and script hash are in `validation-results.json`; the paper includes generated counts.

The PDF build runs in a temporary directory and rejects undefined references/citations and overfull boxes before replacing the deliverable. All pages were rendered for visual inspection after the final build.

### Remaining limits

No engine integration or new performance experiment was run. The validator is a finite model check, not a mechanized proof. No full LPG image, RDF 1.2 mapping, temporal export, or cross-language query equivalence is established. The paper's publication prospects still depend on venue expectations for a narrow representation/design contribution; the revision is not a new peer-review decision.
