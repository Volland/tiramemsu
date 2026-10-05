# SHACL / SPARQL validators for the image definitions

Executable restatement of the image constraints (families D, BB, HO, U, A of `../check_encodings.py`,
`validate`/`decode`) over the *projected row view*, tested for agreement with that Python checker.

**Projection.** Row id k becomes `urn:row:k`; an anchor row `(n, type, class K)` becomes `(row, mg:class, mg:K)`
(in RDF 1.2 this is `?a rdf:reifies <<( ?n rdf:type ?c )>>`; rdflib has no RDF 1.2, so the tested queries run on
the projection). Every other row `(s,p,o)` becomes `(IRI(s), mg:p, IRI/literal(o))`. Every row also gets records
`mg:rowSubject/rowPredicate/rowObject` (duplicates stay distinct there), `rdf:type mg:Row<Family>`, `mg:inStore`. `mg:` = `urn:mg:`.

**Where each constraint lives**
- `image-shapes.ttl` (SHACL core): class sets per family, role subject/object kinds, permitted predicates, every
  Edge has >=1 in and >=1 out (D/BB/HO) or >=1 member (U), exactly one src/tgt/directed(boolean) per Edge/MetaEdge,
  inGraph objects are MetaVertex/MetaEdge, non-reserved literal attributes (A).
- `image-extensionality.rq` (BB, in/out sets); `image-extensionality-member.rq` and `image-acyclic-member.rq`
  (U, `?e mg:member+ ?e`); `image-irreflexive.rq` (HO self-end, A `inGraph` subject != object);
  `image-duplicates.rq` (unique contents, one anchor per fresh node, on the row records);
  `image-row-references.rq` (self/cyclic/dangling row references). Each returns offending stores; empty = accepted.
- `check_validators.py`: projection, runner, agreement test (imports `check_encodings`; that file stays stdlib-only).

**Run** (about 3 min on 12 cores; `--jobs N`, `--candidate-sample N`, `--attributed-sample N`, `--seed S`):
`python3 -m venv .venv && .venv/bin/pip install rdflib pyshacl && .venv/bin/python validators/check_validators.py` (from the paper directory)
(needs rdflib 7.6, pyshacl 0.40). Exit status is non-zero on any disagreement.

**Coverage** (full SHACL + SPARQL on every store; Python `decode` is the oracle)
- Exhaustive: sources(False) 91 stores under D and BB; sources(True) 2,411 under D, BB, HO; 49 recursive member
  candidates and all 256 member subsets (incl. empty/self) under U; all 65,536 role-subset stores under HO; 9 targeted malformed images.
- Sampled (seed 2025): 2,000 of 65,536 attributed stores; seeded single-row mutations (delete, duplicate, retarget,
  rename predicate): 1,200 each for D/BB/HO, 1,800 for A.
Result: 0 disagreements (D 3,702, BB 3,702, HO 69,151, U 305, A 3,805 stores compared).

**Limits.** SHACL/SPARQL express the images over the projected view; they are not engine-enforced and do not
validate RDF 1.2 reification natively. Bounded testing, not a proof. A raw predicate named `class` would collide with
`mg:class` in the projection (untested input); Python-only error paths outside the sets above are not covered.
