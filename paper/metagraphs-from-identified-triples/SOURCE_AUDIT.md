# Source audit — 5 October 2026 (revision 2 additions at the end)

This is a focused literature and metadata audit for the revised metagraph paper, not a complete audit of all active references. Publisher records establish publication details; accessible author manuscripts establish technical claims. An abstract does not establish the absence of a theorem or prove novelty. Document text was treated as evidence, not instructions.

## Sources used in this revision

| Source and direct primary link | Evidence accessed | Use and metadata corrections |
| --- | --- | --- |
| [Chernenkiy et al., 2017: Using metagraph approach for complex domains description](https://ceur-ws.org/Vol-2022/paper52.pdf) | Open full text; structural and agent sections inspected. [Proceedings index](https://ceur-ws.org/Vol-2022/) checked. | Credit the four-class data-model and agent distinction. Correct pages to 342–349; retain all six authors from the paper. |
| [Chernenkiy et al., 2018: Storing Metagraph Model in Relational, Document-Oriented, and Graph Databases](https://ceur-ws.org/Vol-2277/paper17.pdf) | Open full text; especially Sections 3.3 and 4.1. [Proceedings index](https://ceur-ws.org/Vol-2277/) checked. | Credit storage and nested-assertion precedents. Correct pages to 82–89. The displayed statement-ID notation is a reification example, not evidence that standard RDF has occurrence identifiers. |
| [Gelling, Fletcher and Schmidt, 2023: Bridging graph data models](https://arxiv.org/html/2304.13097v1) | Author preprint, especially Sections 4–6, Definition 4.1 and image definitions. | Explicitly credit the statement carrier and image-based method. The paper's metagraph-specific contracts remain the proposed contribution. This preprint was inspected; its later published version is cited separately, without claiming a full comparison of versions. |
| [Sadoughi, Yakovets and Fletcher, 2024: Meta-Property Graphs](https://arxiv.org/html/2410.13813v1) | Author preprint, Sections 3.2–5, Definitions 4.1–4.2. | Add an active citation and source URL. Compare explicit containment with reification; do not imply an encoding of the complete model or preservation of MetaGPML. |
| [Tarassov, Kaganov and Gapanyuk, 2021: The Metagraph Model for Complex Networks](https://link.springer.com/chapter/10.1007/978-3-030-86855-0_10) | Publisher abstract and bibliographic record; full chapter not obtained. | Add volume 12948 and direct URL. Comparison is limited to the abstract's model/calculus/granulation scope. |
| [Vinnikov, Nardid and Gapanyuk, 2025: Metagraph Operations Using Bigraph Representation](https://link.springer.com/chapter/10.1007/978-3-032-03997-2_3) | Publisher abstract, chapter and book records; full chapter not obtained. | Correct book title, add editors, CCIS volume 2641 and URL. Publication year is 2025 despite DAMDID/RCDL 2024 in the conference name. Do not infer Milner bigraph semantics from the title. |
| [Karabulatova et al., 2026: The Metagraph Transformation Algorithm Based on Incidence and Nesting Representation](https://www.pleiades.online/contents/patrec/patrec2_26v36cont.htm) | Publisher issue index; journal full text not obtained. DOI: 10.1134/S1054661826700422. | Confirm volume 36, issue 2 and pages 616–628. Add missing issue number. Technical discussion relies on the separately cited conference precursor below. |
| [Karabulatova et al., DAMDID 2025 conference abstract](https://damdid2025.frccsc.ru/en/conference_program.html) | Official programme, short contribution under Session 4. | Add a separate bibliography entry. Supports the matrix-based incidence/nesting transformation description; does not establish that the 2026 article is textually identical. |
| [Gapanyuk, 2021: proceedings index](https://ceur-ws.org/Vol-2965/) | Primary proceedings metadata; full paper inspected in the preceding revision. | Correct the placeholder page field to 1–7. |

## What changed scientifically

The related-work section now acknowledges earlier statement-image mappings and metagraph storage work rather than merely listing them. The carrier-to-statement-graph correspondence is this manuscript's explanatory observation under its restricted endpoint types, not a new claim attributed to the sources. The Meta-Property Graph comparison identifies a concrete shared modeling choice and a separate query-language obligation. Recent operations papers are compared only at the evidence level actually accessed.

Bibliography changes are confined to inspected records and the newly active citation. The final cleanup removed the historical seed bibliography and original draft from this folder and pruned the active bibliography to cited entries. Earlier checks of ubergraphs, projection semantics, MillenniumDB and memory-system sources are documented in `REVISION_NOTES.md`.

## Remaining work before submission

1. Obtain authorized full texts of the three recent metagraph works and compare definitions, operation semantics and theorem statements. Accessible abstracts cannot settle priority.
2. Audit the remaining active references; resolve version-specific differences when relevant.
3. If stronger systems or agent-memory claims are desired, collect comparative experiments. Citation improvements alone do not provide those results.

## Revision 2 additions

| Source | Evidence accessed | Use |
| --- | --- | --- |
| [Parsonage, Roughan and Nguyen, 2025: Definitions 2.2 and 2.5](https://arxiv.org/html/2509.04543v1) | Author HTML text. | Edges are pairs of subsets of `X` with empty subsets allowed and no disjointness requirement; the simple-path definition is the repetition-permitting one. The Basu–Blanning book text was not accessed; the definitions are cited through this reproduction as well. |
| [Noy and Rector, 2006: Defining N-ary Relations on the Semantic Web](https://www.w3.org/TR/swbp-n-aryRelations/) | Publisher page: W3C Working Group Note, 12 April 2006, intermediate relation node with one property per participant. | Credited as the anchor-plus-role precedent. |
| [TypeDB documentation](https://typedb.com/docs/typeql-reference/data-model/) | Documentation search results and pages; accessed 5 October 2026. | Relations take typed role players and can play roles in other relations. Product documentation, not a peer-reviewed source. |
| Galkin et al., EMNLP 2020 (doi 10.18653/v1/2020.emnlp-main.596) | Crossref metadata (title, authors, pages 7346–7359); full text not read. | Hyper-relational graphs: qualifier pairs on statements. |
| Poulovassilis and Levene, ACM TOIS 12(1):35–68, 1994 (doi 10.1145/174608.174610) | Crossref metadata. A DOI first recalled from memory (`174608.174611`) resolved to a different paper and was not used. | Nested-graph (hypernode) model as a precedent for well-founded nesting. |
| Goertzel, Pennachin and Geisweiller, "The OpenCog Framework" (doi 10.2991/978-94-6239-030-0_1) | Crossref metadata only; full text not read. | Cited for the AtomSpace links-to-links pattern; the technical description rests on the framework's general documentation, not on a read of this chapter. |
| Hernández, Hogan and Krötzsch, 2015; RDF 1.2 Concepts (W3C CR, 7 April 2026); Kulkarni and Michels, 2012; Snodgrass, 1999 | Records reused from the first paper's bibliography (Kulkarni and Michels re-checked by Crossref). | Reification comparison, reifier model, and sequenced temporal referential integrity. |

The uncited `kivela2014multilayer` entry was removed because revision 2 no longer contains the multilayer-network sentence. Everything in the first audit's "Remaining work" list still applies.
