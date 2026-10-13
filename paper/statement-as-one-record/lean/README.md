# Lean mechanization: export boundaries for temporal statement stores

Lean `leanprover/lean4:v4.34.1`, core only (no Mathlib, no network). Build: `cd lean && lake build`. Axiom check: `lake env lean Axioms.lean`. The main theorems use only `propext`, `Classical.choice` and `Quot.sound`. There is no `sorry`, no `axiom` and no `native_decide`.

The paper's proofs are by hand. This development re-proves its combinatorial core. Theorems are named by their LaTeX labels in `statement-as-one-record.tex` (revision 4).

## What is proved

| Paper | Lean | File |
|---|---|---|
| `thm:criterion`: direct export exists iff closed and acyclic; unique; role targets recover content | `direct_export_iff`, `direct_export_unique`, `direct_export_recovers` | `Export.lean` |
| `thm:kernel`: `K(S)` is closed, acyclic, the greatest closed acyclic subset; `K(S) = S` iff closed and acyclic; idempotent; monotone | `K_closed`, `K_acyclic`, `K_greatest`, `K_eq_self_iff`, `K_idem`, `K_mono` | `Kernel.lean` |
| `cor:obstructions`: `K(S) = S ∖ A` | `K_obstructions` | `Kernel.lean` |
| `prop:dual`: reference closure is defined, least closed superset; `K(S) ⊆ K(C_D(S)) ⊆ C_D(S)`; records gained reach outside `S` | `refClosure` | `Kernel.lean` |
| `prop:closure`: snapshots stay closed under C3 inserts and C5 cascading retraction; restricted history acyclic | `snapshot_closed`, `restricted_acyclic` | `History.lean` |
| `prop:history`: history closed; every cycle inside one replay batch; `K(D)` = `D` minus records reaching replay cycles | `history_closed`, `history_cycle`, `history_kernel` | `History.lean` |
| `prop:strict`: under the acyclic-replay rule `D` is closed and acyclic and `K(D) = D` | `history_acyclic` | `History.lean` |
| `prop:bit` (kernel part): kernel membership of old records never changes; closed snapshot kernel = snapshot ∩ `K(D)` | `K_extend`, `K_closed_snapshot` | `Kernel.lean` |
| `thm:temporal`: `T_f(S, 𝒜)` determines `S`, `𝒜` and all eight fields; acyclic for closed acyclic `S`; record vertices have an incoming edge; memberships are unreferenced | `temporal_lossless`, `temporal_acyclic`, `record_has_incoming`, `membership_unreferenced` | `Encoding.lean` |
| `prop:queries` (ii): `q` injective iff contents distinct | `quotes_injective_iff`, `quotes_inj` | `Quoting.lean` |
| `prop:queries` (iii): BGP solutions correspond bijectively through `q` | `matches_forward`, `matches_backward`, `bgp_view` | `Quoting.lean` |
| `prop:rdf12`: reifier encoding lossless with no closure or acyclicity; asserted set recovered iff no shared content across `𝒜` and `S ∖ 𝒜` | `rdf12_lossless`, `rdf12_asserted` | `Encoding.lean` |

`cor:snapshot-queries` is `K_closed_snapshot` followed by `bgp_view`.

## Modelling choices and limits

- **Sets and finiteness.** Record sets are predicates on eids. Finiteness is a list cover (`Covered`), assumed only where the paper uses it: greatest subset, idempotence, obstructions, totality of `q`.
- **Statement Graphs.** Vertices are identified with their labels: an eid for an internal vertex, a target term for a leaf. That makes label injectivity, one edge per role and leaf in-degree hold by construction. Only target existence and acyclicity remain as conditions.
- **Protocol.** C3–C6 are modelled as a history of insert batches with fresh eids. Ordinary and lineage batches reference earlier batches only; replay batches may also reference themselves. Retraction does not change references, so it is absent from the history model. Live-set closure is modelled separately with insert and cascade-retract operations. The batch-local rewriting through `σ` is abstracted to "references in the batch or earlier".
- **Temporal encoding.** Identifiers of time and membership statements are fresh by construction. Position typing (IRI, literal, blank node) and the RDF-star non-isomorphism condition are not modelled. Non-isomorphism is equal quoted triples, which `Quoting.lean` covers.
- **Queries.** `prop:queries` (i), the asserted graph `G_A`, is taken as a definition rather than derived from the statement set. Only conjunctive patterns under set semantics are covered. The SPARQL operator extension is not mechanized.
- **Not mechanized:** `prop:lpg` (LPG branch), the peel algorithm and its `O(n+m)` bound, the `O(|C|)` cost of the acyclic-replay check, and the claim that the rule rejects exactly the patches naming a cascade member. The engine itself is not verified: `engine-trace.json` is checked by `validate.py`, not by Lean.
