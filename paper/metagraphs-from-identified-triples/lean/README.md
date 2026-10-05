# Lean mechanization: encode/decode roundtrip for identified-triple stores

Lean `leanprover/lean4:v4.34.1`, core only (no Mathlib, no network). Build: `cd lean && lake build`.

Proved (`Triples/Roundtrip.lean`, no `sorry`/`axiom`/`native_decide`), for one parametrised development
covering J_D, J_HO (`ho`: role objects may be Edges, self-end forbidden) and BB (`ext`: no two Edges with equal in/out sets):
`dec_enc` (roundtrip up to set-equality of atoms, classes on atoms, roles), `enc_dec` (image stores decode and re-encode to the same row set / a permutation),
`inImage_enc`, `enc_length` (rows = atoms + roles), `enc_rank` (role rows rank 1, referencing only anchor rows of rank 0).

Not proved: models are finite atom lists with canonical naming (row identifiers are determined by atom/content); there is no
"up to fresh renaming of identifiers" bijection. `InImage` is a scan-then-check predicate on the row list (it mirrors the checker, so
it is close to `ValidSource` of the decoded model). No temporal, recursive-member or attributed families. The store is abstracted to
anchor/role rows; literals, IRIs and RDF syntax are not modelled.
