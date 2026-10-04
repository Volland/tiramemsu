# Revision 4 — implemented conformance repairs

3 October 2026. [Rebuilt paper](arxiv/revision-4/layered-bitemporal-graphs-arxiv-preview-v4.pdf). The previous manuscript and counterexample probe are preserved in revisions/v3.

The four reported engine gaps are repaired:

1. Ordinary writes require live statement endpoints. Unknown and retracted endpoints are rejected, including after cardinality-induced cascades.
2. Correction drops foreign memberships and their structural dependents before allocating fresh copies. References to retained copies are rewired, including a patched root object; invalid dropped targets and roots are rejected.
3. Engine-owned lineage keeps historical allocated references; no copies refer to identifiers that were allocated but never inserted.
4. Format 3 installs date guards for insertions and retractions. Dates use the latest transaction record above the committed watermark; same-transaction insert/retract remains allowed.

Existing format-1/2 files migrate atomically when opened by this engine. Historical rows are not rewritten. Older engines refuse format-3 files. Invalid references already present in old files still need validation or sanitation. Raw writers must preserve schema and metadata and follow the transaction protocol; date guards do not replace all engine validations.

Regression tests run on both SQLite hosts. The retained four probes now report `PREVENTED`. The original regression tests failed before the repairs. The paper's abstract, implementation section and conclusion have been updated to reflect these results rather than continuing to claim the gaps remain unrepaired.

See verification report, [artifact commands](artifact/README.md) and retained logs under `review-v4/`. Performance, public archival release and independent proof review remain separate research tasks.
