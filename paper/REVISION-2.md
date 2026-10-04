# Revision 2 — 3 October 2026

The manuscript has been revised and rebuilt following the original major-revision review. The new PDF is layered-bitemporal-graphs-v2.pdf; the original source, bibliography and PDF are preserved in revisions/v1.

| Finding | Revision |
|---|---|
| Missing referents invisible to the core | Require a complete historical carrier; separate missing-reference sanitation from computing the structural core. |
| Correction overclaims | Specify admissible patches, retained-root and atomic-operation contracts; use induced structural retention and substitution only for actual copies; qualify isomorphism for object patches, lineage, and same-transaction rows. |
| Incomplete encoding image | Separate vertex and edge anchors; state role uniqueness, distinct identifiers, non-self constraints, reserved vocabulary and equal role clocks; distinguish static and temporal roundtrips. |
| Compact cycles and n-ary operations | Add acyclicity and non-empty lifetime conditions for LR scheduling; give compact decoding; distinguish snapshot lifting from component-level deletion and correction. |
| Backdated SQL changes past views | Qualify stability by the transaction protocol and report an executable direct-SQL counterexample with triggers enabled. |
| Unsafe bounded-journey pruning | State hop-layer scheduling and frontier invariants; compare finite cases against exhaustive walk enumeration. |
| Novelty and unsupported scale | Attribute the finite-space construction, acknowledge SQL temporal integrity, add a comparison table and notation table, remove unmeasured scale claims. |
| Bibliography and output | Add Barmak's verified book; correct arXiv/category and documentary metadata; render DOI hyperlinks and all six arXiv identifiers. |
| Reproducibility | Retain seeded finite checks, four actual-engine probes, pinned Rust dependencies, build QA, source hashes and a second review. |

An additional scheduling boundary found during revision is now explicit: empty component lifetimes do not affect snapshots but can prevent LR insertion when the referent has not yet been created.

This revision repairs the paper's definitions, statements, proofs, positioning and evidence. The four engine conformance gaps remain documented and reproduced; the database implementation was not changed. A permanent archival release and systems benchmarks remain future work. See the second review and full reference audit.
