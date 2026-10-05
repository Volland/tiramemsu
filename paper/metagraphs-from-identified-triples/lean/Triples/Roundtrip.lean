/-!
Encode/decode roundtrip for directed-hypergraph stores over identified triples.
Lean core only. One parametrised development covers J_D, J_HO and the BB
(extensional) variants via `Variant`.
-/
set_option synthInstance.maxSize 1000

namespace Triples

inductive Class | vertex | edge deriving DecidableEq, Repr
inductive Dir | inn | out deriving DecidableEq, Repr

/-- A store row: anchor `(n_a, type, C)` or role `(h_a, in|out, h_b)`. -/
inductive Row (A : Type) | anchor (a : A) (c : Class) | role (a : A) (d : Dir) (b : A)
  deriving DecidableEq

/-- `ho`: role objects may be Edge atoms (self-end forbidden). `ext`: extensionality. -/
structure Variant where
  ho : Bool
  ext : Bool

def J_D : Variant := ⟨false, false⟩
def J_HO : Variant := ⟨true, false⟩
def J_BB : Variant := ⟨false, true⟩
def J_HO_BB : Variant := ⟨true, true⟩

variable {A : Type} [DecidableEq A]

/-- A finite model: atom list, class function, role triples. -/
structure Model (A : Type) where
  atoms : List A
  cls : A → Class
  roles : List (A × Dir × A)

/-- Well-formedness shared by sources and decoded stores. -/
def Wf (v : Variant) (atoms : List A) (cls : A → Class) (rs : List (A × Dir × A)) : Prop :=
  (∀ r ∈ rs, r.1 ∈ atoms ∧ r.2.2 ∈ atoms ∧ cls r.1 = .edge ∧
      (v.ho = true → r.1 ≠ r.2.2) ∧ (v.ho = false → cls r.2.2 = .vertex)) ∧
  (∀ e ∈ atoms, cls e = .edge →
      (∃ r ∈ rs, r.1 = e ∧ r.2.1 = Dir.inn) ∧ (∃ r ∈ rs, r.1 = e ∧ r.2.1 = Dir.out)) ∧
  (v.ext = true → ∀ e ∈ atoms, ∀ e' ∈ atoms, cls e = .edge → cls e' = .edge →
      (∀ b ∈ atoms, (((e, Dir.inn, b) ∈ rs ↔ (e', Dir.inn, b) ∈ rs) ∧
                     ((e, Dir.out, b) ∈ rs ↔ (e', Dir.out, b) ∈ rs))) → e = e')

instance [DecidableEq A] (v : Variant) (atoms : List A) (cls : A → Class) (rs : List (A × Dir × A)) :
    Decidable (Wf v atoms cls rs) := by unfold Wf; infer_instance

def ValidSource (v : Variant) (M : Model A) : Prop :=
  M.atoms.Nodup ∧ M.roles.Nodup ∧ Wf v M.atoms M.cls M.roles

/-- Same atoms, same classes on atoms, same roles (as sets). -/
def Equiv (M N : Model A) : Prop :=
  (∀ a, a ∈ M.atoms ↔ a ∈ N.atoms) ∧ (∀ a ∈ M.atoms, M.cls a = N.cls a) ∧
  (∀ r, r ∈ M.roles ↔ r ∈ N.roles)

theorem Wf.congr {v : Variant} {at' at₂ : List A} {c₁ c₂ : A → Class}
    {r₁ r₂ : List (A × Dir × A)}
    (hat : ∀ a, a ∈ at' ↔ a ∈ at₂) (hc : ∀ a ∈ at', c₁ a = c₂ a)
    (hr : ∀ r, r ∈ r₁ ↔ r ∈ r₂) (h : Wf v at' c₁ r₁) : Wf v at₂ c₂ r₂ := by
  obtain ⟨h1, h2, h3⟩ := h
  refine ⟨?_, ?_, ?_⟩
  · intro r hrm
    obtain ⟨a1, a2, a3, a4, a5⟩ := h1 r ((hr r).2 hrm)
    have e1 : c₁ r.1 = c₂ r.1 := hc _ a1
    have e2 : c₁ r.2.2 = c₂ r.2.2 := hc _ a2
    exact ⟨(hat _).1 a1, (hat _).1 a2, e1 ▸ a3, a4, fun hh => e2 ▸ a5 hh⟩
  · intro e he hce
    have he' := (hat e).2 he
    obtain ⟨⟨r, hr1, hr2, hr3⟩, ⟨s, hs1, hs2, hs3⟩⟩ := h2 e he' (by rw [hc e he']; exact hce)
    exact ⟨⟨r, (hr r).1 hr1, hr2, hr3⟩, ⟨s, (hr s).1 hs1, hs2, hs3⟩⟩
  · intro hx e he e' he' hce hce' hb
    have he1 := (hat e).2 he
    have he2 := (hat e').2 he'
    exact h3 hx e he1 e' he2 (by rw [hc e he1]; exact hce) (by rw [hc e' he2]; exact hce')
      (fun b hb' => by
        have := hb b ((hat b).1 hb')
        simp only [hr] at this ⊢; exact this)

/-! ## Scanning a store -/

def anchors (G : List (Row A)) : List (A × Class) :=
  G.filterMap fun | .anchor a c => some (a, c) | _ => none

def rolesOf (G : List (Row A)) : List (A × Dir × A) :=
  G.filterMap fun | .role a d b => some (a, d, b) | _ => none

def atomsOf (G : List (Row A)) : List A := (anchors G).map Prod.fst

def clsOf (G : List (Row A)) (a : A) : Class := ((anchors G).lookup a).getD .vertex

theorem mem_anchors {G : List (Row A)} {a c} : (a, c) ∈ anchors G ↔ Row.anchor a c ∈ G := by
  simp only [anchors, List.mem_filterMap]
  constructor
  · rintro ⟨x, hx, h⟩; cases x <;> simp_all
  · intro h; exact ⟨_, h, rfl⟩

theorem mem_rolesOf {G : List (Row A)} {a d b} : (a, d, b) ∈ rolesOf G ↔ Row.role a d b ∈ G := by
  simp only [rolesOf, List.mem_filterMap]
  constructor
  · rintro ⟨x, hx, h⟩; cases x <;> simp_all
  · intro h; exact ⟨_, h, rfl⟩

theorem mem_atomsOf {G : List (Row A)} {a} : a ∈ atomsOf G ↔ ∃ c, Row.anchor a c ∈ G := by
  unfold atomsOf
  rw [List.mem_map]
  constructor
  · rintro ⟨⟨a', c⟩, h, rfl⟩; exact ⟨c, mem_anchors.1 h⟩
  · rintro ⟨c, h⟩; exact ⟨(a, c), mem_anchors.2 h, rfl⟩

theorem lookup_of_mem {l : List (A × Class)} (hn : (l.map Prod.fst).Nodup) {a c}
    (h : (a, c) ∈ l) : l.lookup a = some c := by
  induction l with
  | nil => simp at h
  | cons x t ih =>
    obtain ⟨y, z⟩ := x
    simp only [List.map_cons, List.nodup_cons, List.mem_map] at hn
    rcases List.mem_cons.1 h with h | h
    · simp only [Prod.mk.injEq] at h; obtain ⟨rfl, rfl⟩ := h; simp [List.lookup]
    · have hne : y ≠ a := fun e => hn.1 ⟨(a, c), h, e.symm⟩
      simp [List.lookup, show (a == y) = false by simpa using hne.symm, ih hn.2 h]

theorem clsOf_eq {G : List (Row A)} (hn : (atomsOf G).Nodup) {a c}
    (h : Row.anchor a c ∈ G) : clsOf G a = c := by
  simp [clsOf, lookup_of_mem hn (mem_anchors.2 h)]

/-! ## Encode, image, decode -/

def enc (M : Model A) : List (Row A) :=
  M.atoms.map (fun a => .anchor a (M.cls a)) ++ M.roles.map (fun r => .role r.1 r.2.1 r.2.2)

/-- Image predicate (J_D / J_HO / BB, by `v`): rows are anchors/roles by type; scanned
atoms have exactly one anchor; roles reference anchored atoms of the right classes; edge
in/out coverage; no duplicate rows or roles; extensionality for BB. -/
def InImage (v : Variant) (G : List (Row A)) : Prop :=
  G.Nodup ∧ (atomsOf G).Nodup ∧ (rolesOf G).Nodup ∧ Wf v (atomsOf G) (clsOf G) (rolesOf G)

instance (v : Variant) (G : List (Row A)) : Decidable (InImage v G) := by
  unfold InImage; infer_instance

def dec (v : Variant) (G : List (Row A)) : Option (Model A) :=
  if InImage v G then some ⟨atomsOf G, clsOf G, rolesOf G⟩ else none

theorem dec_of_inImage {v : Variant} {G : List (Row A)} (h : InImage v G) :
    dec v G = some ⟨atomsOf G, clsOf G, rolesOf G⟩ := by simp [dec, h]

/-! ## Basic facts about `enc` -/

theorem anchors_enc (M : Model A) : anchors (enc M) = M.atoms.map (fun a => (a, M.cls a)) := by
  simp [anchors, enc, List.filterMap_append, List.filterMap_map, Function.comp_def]

theorem rolesOf_enc (M : Model A) : rolesOf (enc M) = M.roles := by
  simp [rolesOf, enc, List.filterMap_append, List.filterMap_map, Function.comp_def]

theorem atomsOf_enc (M : Model A) : atomsOf (enc M) = M.atoms := by
  simp [atomsOf, anchors_enc, Function.comp_def]

theorem clsOf_enc (M : Model A) (h : M.atoms.Nodup) {a} (ha : a ∈ M.atoms) :
    clsOf (enc M) a = M.cls a :=
  clsOf_eq (by rwa [atomsOf_enc]) (by simp only [enc, List.mem_append, List.mem_map, Row.anchor.injEq]; exact Or.inl ⟨a, ha, rfl, rfl⟩)

/-- (4) Row count. -/
theorem enc_length (M : Model A) : (enc M).length = M.atoms.length + M.roles.length := by
  simp [enc]

theorem enc_nodup {M : Model A} (h1 : M.atoms.Nodup) (h2 : M.roles.Nodup) : (enc M).Nodup := by
  unfold enc
  rw [List.nodup_append]
  refine ⟨?_, ?_, ?_⟩
  · exact List.Pairwise.map _ (fun x y hne h => hne (by cases h; rfl)) h1
  · exact List.Pairwise.map _ (fun x y hne h => hne (by
      obtain ⟨a, d, b⟩ := x; obtain ⟨a', d', b'⟩ := y; cases h; rfl)) h2
  · intro x hx y hy hxy
    simp only [List.mem_map] at hx hy
    obtain ⟨_, _, rfl⟩ := hx; obtain ⟨_, _, rfl⟩ := hy; cases hxy

/-- (3) The encoding of a valid source lies in the image. -/
theorem inImage_enc {v : Variant} {M : Model A} (h : ValidSource v M) : InImage v (enc M) := by
  obtain ⟨h1, h2, h3⟩ := h
  refine ⟨enc_nodup h1 h2, by rwa [atomsOf_enc], by rwa [rolesOf_enc], ?_⟩
  rw [atomsOf_enc, rolesOf_enc]
  exact h3.congr (fun _ => Iff.rfl) (fun a ha => (clsOf_enc M h1 ha).symm) (fun _ => Iff.rfl)

/-- (1) Roundtrip: decoding the encoding of a valid source gives an equivalent model. -/
theorem dec_enc {v : Variant} {M : Model A} (h : ValidSource v M) :
    ∃ M', dec v (enc M) = some M' ∧ Equiv M' M := by
  refine ⟨⟨atomsOf (enc M), clsOf (enc M), rolesOf (enc M)⟩, dec_of_inImage (inImage_enc h), ?_⟩
  refine ⟨?_, ?_, ?_⟩
  · intro a; simp [atomsOf_enc]
  · intro a ha; exact clsOf_enc M h.1 (by simpa [atomsOf_enc] using ha)
  · intro r; simp [rolesOf_enc]

/-- Row-set equality of stores. -/
def StoreEquiv (G H : List (Row A)) : Prop := ∀ r, r ∈ G ↔ r ∈ H

/-- (2) Every store in the image decodes to a model whose encoding is the same row set
(and, rows being duplicate-free, a permutation of the store). -/
theorem enc_dec {v : Variant} {G : List (Row A)} (h : InImage v G) :
    ∃ M, dec v G = some M ∧ StoreEquiv (enc M) G ∧ (enc M).Perm G := by
  obtain ⟨hG, hat, hro, hwf⟩ := h
  have hI : InImage v G := ⟨hG, hat, hro, hwf⟩
  refine ⟨⟨atomsOf G, clsOf G, rolesOf G⟩, dec_of_inImage hI, ?_⟩
  have hse : StoreEquiv (enc ⟨atomsOf G, clsOf G, rolesOf G⟩) G := by
    intro r
    cases r with
    | anchor a c =>
      simp only [enc, List.mem_append, List.mem_map, Row.anchor.injEq]
      constructor
      · rintro (⟨x, hx, rfl, rfl⟩ | ⟨x, _, h⟩)
        · obtain ⟨c', hc'⟩ := mem_atomsOf.1 hx
          rwa [clsOf_eq hat hc']
        · cases h
      · intro hm
        left
        exact ⟨a, mem_atomsOf.2 ⟨c, hm⟩, rfl, (clsOf_eq hat hm)⟩
    | role a d b =>
      simp only [enc, List.mem_append, List.mem_map, Row.anchor.injEq, Row.role.injEq]
      constructor
      · rintro (⟨x, _, h⟩ | ⟨x, hx, h1, h2, h3⟩)
        · cases h
        · obtain ⟨x1, x2, x3⟩ := x
          simp only at h1 h2 h3; subst h1 h2 h3
          exact mem_rolesOf.1 hx
      · intro hm
        right
        exact ⟨(a, d, b), mem_rolesOf.2 hm, rfl, rfl, rfl⟩
  exact ⟨hse, (List.perm_ext_iff_of_nodup
    (enc_nodup hat hro) hG).2 hse⟩

/-! ## Rank: role rows reference only anchor rows -/

def rank : Row A → Nat | .anchor .. => 0 | .role .. => 1

/-- Rows a row depends on: a role references the anchors of its two ends. -/
def refs (cls : A → Class) : Row A → List (Row A)
  | .anchor .. => []
  | .role a _ b => [.anchor a (cls a), .anchor b (cls b)]

/-- (5) Every row of `enc M` has rank ≤ 1, and every row it references is an anchor
(rank 0) that is itself a row of `enc M`. -/
theorem enc_rank {v : Variant} {M : Model A} (h : ValidSource v M) :
    ∀ r ∈ enc M, rank r ≤ 1 ∧ ∀ s ∈ refs M.cls r, s ∈ enc M ∧ rank s = 0 := by
  intro r hr
  simp only [enc, List.mem_append, List.mem_map] at hr
  rcases hr with ⟨a, _, rfl⟩ | ⟨x, hx, rfl⟩
  · simp [rank, refs]
  · obtain ⟨hr1, -, -⟩ := h.2.2
    obtain ⟨h1, h2, -⟩ := hr1 x hx
    simp only [rank, refs, List.mem_cons, List.not_mem_nil, or_false]
    refine ⟨by omega, ?_⟩
    rintro s (rfl | rfl) <;> simp only [enc, List.mem_append, List.mem_map, rank, and_true]
    · exact Or.inl ⟨x.1, h1, rfl⟩
    · exact Or.inl ⟨x.2.2, h2, rfl⟩

end Triples
