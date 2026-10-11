# Referee report — The Statement as One Record

Reviewed 11 October 2026. Manuscript: revision 1, 11-page PDF and its LaTeX source. Locations below refer to `statement-as-one-record.tex` in this revision.

## 1. Verdict and rating

**Major revision; not ready for submission as a full research paper. Overall publication readiness: 4/10.** The manuscript explains an interesting engineering design clearly and gives useful vocabulary for statement identity, annotations, and temporal integrity. However, its headline claim—that every supported snapshot becomes a Statement Graph in the OneGraph image—is not established for the documented engine. Historical lineage defeats full live-reference closure, and correction can create reference cycles. The claimed exact image conditions also need explicit encoding functions and restrictions. The two short invariants are useful but substantially overlap the author's first paper; the storage experiment is inherited and does not isolate the causal effect claimed. A corrected, tightly scoped workshop/design paper is plausible. A full research paper needs a distinct result about interoperability or update semantics, with reproducible validation.

These are subjective referee assessments, not acceptance probabilities or an official venue ranking.

| Dimension | Rating | Reason |
| --- | ---: | --- |
| Problem relevance | 8/10 | Identity and updates across RDF and property-graph views are useful problems. |
| Exposition | 7/10 | Accessible examples and a readable structure; several precise-sounding claims exceed their definitions. |
| Novelty as a standalone paper | 3/10 | Row encoding and the two invariants are elementary; substantial overlap with the site article and companion papers. |
| Technical soundness as written | 4/10 | Conditional arguments are mostly repairable, but their application to the engine and the exact-image corollary are overbroad. |
| Evidence and reproducibility | 3/10 | Accessible code and documentation, but no validation artifact specific to these claims and incomplete benchmark provenance. |
| Publication readiness | 4/10 | Requires changes to the central claims, not just editing. |

The strongest aspect is the operational treatment of statement occurrences. The weakest is treating the choice of a row layout as if it independently supplies semantic guarantees that actually come from a write protocol.

## 2. Numbered findings

### 1. [Blocking] Full snapshot closure excludes the engine's own lineage records

**Location:** abstract; §4, lines 308–328; C3 and Theorem 3, lines 352–389; Table 1.

The theorem is reasonable for a protocol that enforces liveness for **every** inserted reference. The manuscript defines C3 for an “ordinary write,” then applies its conclusion to the whole store, including supersession lineage.

Counterexample from the documented operation: assert `e1` at transaction 1; supersede it at transaction 2, producing `e2` and a live record `l = (e2, sys:supersedes, e1)`. Then `l ∈ D@2`, `l ≺ e1`, but `e1 ∉ D@2`. Consequently the defined `SG(D@2)` lacks an internal vertex for a referenced eid. This is deliberate historical linkage, not an engine defect.

Evidence: `lat.md/time-model.md`, lines 107 and 155–181. I checked the same exception in **tag v0.4.0**, commit `6e7961c0378effc1793896395c464c8af460566b`, so this is not an inference from unrelated uncommitted work. The first companion paper already distinguishes full closure from structural closure (`paper/layered-bitemporal-graphs.tex`, lines 507–523).

**Fix:** separate structural references from historical references. State closure for the former, and define the representation of the latter. Options include a rigorously selected structural fragment, or an export carrier retaining historical referents with an explicit asserted/unasserted distinction. Merely weakening the theorem to structural closure does **not** repair the original `SG` translation, which still follows both endpoints of every included row. Propagate the chosen restriction through the abstract, corollary, table, and conclusion.

### 2. [Blocking] Supported correction can create reference cycles

**Location:** Proposition 4 and following paragraph, lines 395–412; Table 1; §7, lines 558–562.

Proposition 4 is correct under its stated premise: every new row references only earlier rows. That premise does not cover atomic replay with preallocated identifiers. The statement that supersede explicitly rejects reference cycles contradicts the release documentation.

Concrete witness: start with `e1 = (:a, :p, :b)` and `e2 = (e1, :q, 1)`. Supersede `e1` with its object patched to `e2`. Substitution yields `e1' = (:a, :p, e2')` and `e2' = (e1', :q, 1)`: a two-cycle with no direct self-reference. The v0.4.0 test documentation explicitly describes this outcome under **Correction Remaps Patched References** (`lat.md/tests.md`, lines 1310–1316). The companion paper also explicitly excludes such patches from its acyclicity proposition, lines 604–624.

**Fix:** present acyclicity as a restricted protocol guarantee, or a checked precondition for export. Add an operation table covering assert/create, confirm, membership, import, and supersede. Distinguish preservation of an existing DAG from acceptance of arbitrary corrections. A faithful finite encoding into a DAG using the same eid reference edges cannot preserve this cycle; the paper must state what is rejected or transformed. Do not attribute the entire exception to raw SQL.

### 3. [Major] Corollary 2 needs a separate, fully specified image encoding

**Location:** §4, lines 313–328; C9, lines 433–442.

The raw `SG` translation and a surface-model encoding are different functions. The manuscript currently moves between them using “up to” and “form an LPG image.”

The external definitions impose typed leaves and reserved statement forms in addition to graph shape. In particular, the LPG image types predicates as property names or label sets, and RDF-star membership statements have special restrictions. [Gelling et al., Definitions 5.1–5.4](https://arxiv.org/html/2304.13097v1#S5).

Two checks expose the missing specifications:

- **LPG typing:** `r = (:a, :p, :b)` has no eid endpoint, so the proposed condition holds vacuously. Yet the raw graph has IRI leaves, not the specified LPG node/label types. A conversion is needed. If `:p` also occurs in a literal-valued row or as a subject, one shared leaf may require different role-dependent conversions. Define these conversions and any leaf splitting before asserting an iff.
- **Membership policy:** the engine permits annotations on membership rows and statement-valued graph names (`lat.md/data-model.md`, lines 124–126). Simply equating `sys:inGraph` with the target's reserved membership predicate will not preserve those cases. Treating all original rows as ordinary triples can instead preserve their structure, but does not automatically preserve their named-graph interpretation.

**Fix:** define `E_RDFstar` and `E_LPG` independently of `SG`. Specify reserved vocabulary, fresh identifiers, injective term conversion, statement roles, and synthetic memberships. Restrict each iff to that exact encoding. Provide necessity witnesses and both directions of the proof. For RDF-star, the no-duplicate criterion is plausible for an acyclic, closed carrier after an injective conversion and fresh membership construction; it is not a sufficient description of the unrestricted engine export. C9's “lossy exactly” claim must be limited to a specified direct mapping: an encoding using auxiliary nodes can retain information outside that mapping.

### 4. [Major] Theorem 3 needs explicit transaction premises and a corrected recurrence

**Location:** Definition 2, lines 196–202; proof of Theorem 3, lines 373–388.

The treatment of a new annotation whose target is retracted later in the same transaction is substantively correct: C5 removes the annotation too, and `a=d=t` gives an empty committed lifetime. Do **not** change `a≤d` to `a<d`.

However, the proof's remove-then-add description can re-add a row inserted and retracted in the same transaction. Write the actual recurrence:

`D@t = (D@(t−1) ∪ I_t) \ R_t`,

where `I_t` includes all rows inserted during transaction `t`, and `R_t` includes all rows retracted during it, including new ones. Explicitly require an initially closed state, chronological transaction allocation, inserts with `a=t`, retractions with `d=t`, atomic commit, and validation at each operation or a specified atomic replay rule. C4 alone states immutability, not all these timing rules.

The following claim that `D@(t−1)` contains the “whole structure” is false for rows born and retracted at `t`; they occur in no committed snapshot.

**Fix:** distinguish operation-time liveness from committed-snapshot visibility and narrow the explanatory sentence to records live before the retracting transaction. Retain the conditional invariant, subject to finding 1.

### 5. [Major] Projection is not yet an RDF/SPARQL semantics

**Location:** C8, lines 423–431; §3, lines 218–239.

`π_spo(D@t)` is not by itself a complete RDF view: endpoints can be eids, transaction identities are underspecified, and named-graph membership and reifier triples require interpretation. Projection also discards identity and the four time columns even when every triple is unique. The sentence that it loses information “exactly” when occurrences repeat is false for the defined record store.

RDF graph set semantics must also be distinguished from SPARQL solution multiplicities. Deduplicating duplicate stored triples does not make every SPARQL answer a set. An occurrence store is richer than an ordinary bag of triples because it carries independently addressable occurrences and their annotations.

**Fix:** replace the information-loss claim with “projection merges distinct occurrences when they share a triple.” Define the surface view, the relevant query fragment, and its multiplicity behavior. Keep occurrence identity, triple-term identity, and result multiplicity as separate concepts.

### 6. [Major] The RDF-star correspondence does not establish RDF 1.2 coverage

**Location:** §3 annotation example; §4 corollary; §6 annotation-cost paragraph; related work.

The paper uses a historical RDF-star image and modern RDF 1.2 reifiers in one narrative, without an explicit bridge. RDF 1.2 allows reification of an unasserted proposition; a reifier is not the same thing as an asserted occurrence. [W3C RDF 1.2 Concepts, §1.5](https://www.w3.org/TR/2026/CR-rdf12-concepts-20260407/#section-triple-terms-reification).

Counterexample to an unrestricted interchange claim: an RDF graph describes a reified allegation without asserting the allegation itself. The proposed “live bearer required” policy cannot simply assert the bearer and preserve that meaning.

**Fix:** state that the result concerns the cited RDF-star image. Add a separate RDF 1.2 mapping or declare an asserted-occurrence subset. Explain reifier cardinality, unasserted terms, nested annotation, and the meaning of exporting an eid as a resource. A short limitation here is better than implying full standards coverage.

### 7. [Major] The independent research contribution is currently too small

**Location:** contribution list, lines 114–123; §§4–5 and related work.

Proposition 1 is a direct encoding check. Proposition 4 is the standard decreasing-order argument. Theorem 3 is a useful operational invariant, but the first companion already gives structural closure, lifetime nesting, cascade minimality, and a more careful acyclicity result. The second companion and the 6 October site article already discuss the identified-triple/statement-vertex correspondence. The ten-item table is useful synthesis, not ten independent discoveries.

**Fix:** add an explicit contribution-delta table against both companions and the site article. Choose a genuinely separate question, preferably exact export boundaries or updates through occurrence-collapsing views. Otherwise consolidate this material into a companion paper or present it as a design/experience paper. More theorem names or a Lean transcription of the elementary lemmas would not resolve the novelty issue.

### 8. [Major] Table 2 is exploratory layout evidence, not an isolated identifier cost

**Location:** §7, lines 513–544; `bench/eid-vs-reifier/bench.py`.

The reused measurements are clearly labeled synthetic, but the manuscript does not say they are reproduced from the earlier article. The script changes several factors simultaneously: rowid versus clustered primary-key storage, index organization, annotation encoding, and join shape. Neither layout includes the proposed four time columns, temporal triggers, cascade, or full history indexing. Layout B's table has `(s,p,o)` and no graph column, despite being called “quads.” Three covering permutations do not equal identical physical storage: A has its rowid table plus three indexes, while B's primary table supplies one permutation.

The script generates 700,000 base triples, 50,000 confidence annotations, and 500 source annotations. Thus A initially has **750,500 rows**; B has 801,000 rows in its main table plus 50,500 triple-term rows. “750,000 triples” is at best an unexplained approximation of one representation, not a shared physical count.

Further limitations: `synchronous=OFF`; aggregate mean timings without dispersion; A always timed before B; different write targets in the two layouts; no general result-equivalence validation (line 113 suppresses comparison for the 2,000-iteration queries and only the last result is retained). The set-join case measures a particular canonical-eid implementation on data without parallel base occurrences. It cannot establish an unavoidable cost of occurrence semantics.

**Fix:** call the table “reproduced exploratory measurements of two simplified SQLite layouts,” cite the earlier article, provide exact logical and physical counts, and remove the causal “identifier costs” and general “pays for itself” conclusions. For a full systems paper, separately control storage layout and encoding, validate all results outside timing, report environment/durability settings and distributions, vary annotation/duplicate density, and measure actual temporal operations. No benchmark was rerun for this review.

### 9. [Major] C1–C10 mixes representation, policy, capability, and implementation

**Location:** §5 introduction and Table 1, lines 335–467.

The list is not an independent or complete axiom system. C1–C2 describe representation; C3–C5 a write discipline; C6 a conditional consequence; C7 combines temporal representation with a permissive policy; C8–C9 describe capabilities; C10 is optional schema. Identity uniqueness, term-label injectivity, interval validity, freshness, and transaction chronology sit elsewhere. “C3 checked on every write” and “C6 consequence of C3” are too strong for the engine.

**Fix:** keep the ten labels for readability, but call them design commitments and enforcement rules. Add columns for formal scope, enforcing layer, exception, and evidence. Separate the abstract `p∈I` type from its concrete SQL/API enforcement; an integer column alone does not establish IRI typing. State that file-level trigger guarantees assume an intact schema and metadata. Distinguish an immutable statement occurrence from the mutable retraction field.

### 10. [Major] Logical structure and physical layout are compared at different levels

**Location:** §6, lines 470–503; §7 freedom of layout; conclusion.

A logical graph vertex with three role edges does not require four independently written physical records. A Statement Graph implementation can choose the same tuple layout and transactional guarantees. Conversely, storing one tuple does not cause cascade policy, referential liveness, bitemporality, or cross-language correctness. Those are additional design decisions.

“One insert” is true for a primitive base-row insertion, not for schema effects, memberships, indexes, or a general user operation. “One update” describes one row's retraction, not the cascade. Logical RDF reification components are likewise not a lower bound on physical storage operations.

**Fix:** frame the paper as one implementation and operational discipline for a statement-centric model. Replace inherent-benefit language with measured or explicitly qualified implementation claims. Explain index maintenance where making physical cost claims.

### 11. [Major] Neptune claims need narrower attribution and the missing vision discussion

**Location:** title, abstract, §§2.3 and 6, Table 3.

The inspected demo paper supports OneGraph concepts in Neptune Analytics, relationship/property statements, identity coexistence, and openCypher updates. It does not establish that Neptune implements exactly the formal Statement Graph axioms, their entire image class, or the proposed physical contrast. [Schmidt et al., §§2–4](https://ceur-ws.org/Vol-3828/paper29.pdf).

I also located the previously unread OneGraph vision paper on the journal's site. Its §3.7 explicitly discusses ambiguous deletion of duplicate occurrences and attached properties. That is a direct motivation for this paper's operational question, and should be acknowledged. [Lassila et al., §3.7](https://www.semantic-web-journal.net/system/files/swj3273.pdf).

**Fix:** use “not specified in the cited publication” instead of “semantics not published.” Attribute production claims to OneGraph concepts rather than equating the product with the full formal model. Replace the title's “Neptune Statement Graphs” with “Statement Graphs,” and discuss the vision paper's update challenge as prior problem formulation. No product benchmark against Neptune is available here.

### 12. [Minor] Several examples and terms need exact boundaries

**Location:** introduction; Definition 2; flight example; C7–C8; §6.

Concrete edits:

- “RDF gives every resource a global IRI” overlooks blank nodes. Say that RDF supports global IRI identification.
- Define `Lit` and transaction terms explicitly. The parenthetical TX exception currently sits outside the mathematical subject domain; confirm also uses a transaction as object.
- Distinguish two valid-time employment episodes from two transaction-time belief lifetimes. Non-overlapping real-world episodes can both be believed now.
- The March/January example motivates two clocks, but does not by itself demonstrate why an annotation's valid interval should extend outside the bearer's. Use a dated dispute about an expired contract.
- The SPARQL flight query fixes the two airports; the Cypher query matches every route. Bind identical endpoints and explain any occurrence multiplicity difference.
- Conventional RDF reification uses four description triples; attaching `distance` requires another triple. State exactly which triples are counted.
- Define “record,” “occurrence,” “triple content,” “statement vertex,” and “row,” then use each consistently.
- Replace “common case pays nothing,” “Annotation is cheap,” and “pays for itself” with bounded statements about this implementation and workload.

## 3. Publication plan

**Recommended thesis:** identify exactly which operational statement stores admit a faithful Statement Graph export, and what must change when history or correction breaks those conditions.

Suggested title: **The Statement as One Record: Temporal Integrity and the Limits of Statement-Graph Interoperability**.

Suggested central sentence, conditional on the revised definitions and proofs:

> We characterize a closed, acyclic fragment of an occurrence-based temporal store that admits a faithful Statement Graph representation, and identify where historical lineage, correction cycles, and dataset membership require a different encoding.

Do not claim the characterization is proved until the encoding and its reverse direction exist.

| Priority | Deliverable | Completion criterion |
| --- | --- | --- |
| P0 | Repair closure and acyclicity scope | The lineage and replay-cycle witnesses are handled explicitly; abstract, body, and conclusion agree. |
| P0 | Define image encodings | Typed conversion, reserved terms, membership, graph names, and statement roles are explicit; no unexplained “up to.” |
| P0 | State evidence provenance | Release commit pinned; inherited measurements labeled; unread sources and untested claims disclosed. |
| P1 | Establish a distinct result | A precise export/roundtrip boundary or a query-update result beyond the two companion papers. |
| P1 | Build a focused validation artifact | Independent checks of image membership and decoded results, with positive cases and small counterexamples. |
| P1 | Reorganize the manuscript | Problem → model and policies → exact encoding → preservation/boundaries → validation → limitations. |
| P2 | Add new systems experiments if pursuing a full systems paper | Controlled comparisons with equivalent tasks and temporal workloads, not just inherited point timings. |

A useful candidate result is an exact roundtrip for the selected structural fragment, paired with a classification of failures outside it. If updates are the focus, define a surface triple-deletion policy and prove that its occurrence-level implementation has the intended projection and cascade effects. Compare “delete one occurrence” and “delete all matching occurrences”; state what the surface view cannot determine without extra policy. These are proposed research obligations, not results established by this review.

Validation should include same-transaction insertion/retraction, lineage to historical records, the correction-generated two-cycle, annotations of membership, statement graph names, reused predicate IRIs across roles, duplicate occurrences with different annotations, and unasserted RDF 1.2 reifiers. Use independently specified expected outcomes; finite checks support proofs and do not replace them.

For a **workshop or design paper**, a narrow correct account with a small executable artifact is the most realistic next step. For a **full research paper**, editorial repairs alone will not suffice: supply the independent result and convincing validation. If that result remains largely covered by the companions, consolidation is preferable to a third overlapping paper. No current venue deadlines or acceptance odds were assessed.

## 4. Sources and scope of this review

- Read the entire manuscript source, its bibliography, source audit and review brief; confirmed that the accompanying PDF has 11 pages. This was a scientific/content review, not a visual typesetting audit.
- Read relevant local model, time, storage, and test documentation, the benchmark script, the earlier site articles, and the companion-paper passages relevant to overlap and correctness. Checked the decisive lineage and correction-cycle documentation against **v0.4.0**, rather than relying only on the changing workspace.
- Independently read the raw [Gelling et al. preprint definitions and mappings](https://arxiv.org/html/2304.13097v1), including Definitions 4.1 and 5.1–5.4. Its component-level OneGraph formulation and injective labeling support the manuscript's corresponding background corrections. The separate DASFAA full text was not obtained; this review does not claim to have checked it.
- Independently inspected the [Neptune demo paper](https://ceur-ws.org/Vol-3828/paper29.pdf), the relevant [OneGraph vision sections](https://www.semantic-web-journal.net/system/files/swj3273.pdf), and the cited dated [RDF 1.2 Concepts snapshot](https://www.w3.org/TR/2026/CR-rdf12-concepts-20260407/). The journal-hosted vision PDF has prepublication page furniture; its text was checked, not its equivalence to every final-publication detail.
- No Rust code was changed or audited, no engine tests or performance experiments were run, and the manuscript was left unchanged. Counterexamples here are formal constructions or deductions from the pinned release documentation, not newly executed engine traces. The source audit's assertions that supersede rejects cycles and that all live references are closed should be corrected during revision.
