import StatementExport.Export

/-!
# What queries see

The quoting map `q` sends a term `t` to the leaf `f(t)` and an eid to the quoted triple of
the quotes of its subject and object. It is defined as a relation, so no recursion on
rank is needed; it is total on closed, acyclic, finite sets and always functional. This
file proves `prop:queries` (ii) and (iii) for basic graph patterns under set semantics,
taking (i), the asserted graph `G_A`, as the definition of what a time-unaware consumer
sees.
-/

namespace StatementExport

variable {T E L V : Type}

/-- RDF-star values: leaves and quoted triples. -/
inductive QT (L : Type) where
  | leaf (l : L)
  | triple (s : QT L) (p : L) (o : QT L)

section
variable (c : E → Content T E) (f : T → L) (S : E → Prop)

/-- `Quotes n q`: `q` is the quoting-map image of the node `n` over `S`. -/
inductive Quotes : Node T E → QT L → Prop
  | term (t : T) : Quotes (.term t) (.leaf (f t))
  | eid {e : E} {qs qo : QT L} : S e → Quotes (c e).s qs → Quotes (c e).o qo →
      Quotes (.eid e) (.triple qs (f (c e).p) qo)

end

variable {c : E → Content T E} {f : T → L} {S : E → Prop}

theorem quotes_functional {n : Node T E} {q q' : QT L} (h : Quotes c f S n q)
    (h' : Quotes c f S n q') : q = q' := by
  induction h generalizing q' with
  | term => cases h'; rfl
  | eid _ _ _ ihs iho => cases h' with
    | eid _ hs ho => rw [ihs hs, iho ho]

theorem quotes_node {n : Node T E} (h : ∀ x, n = .eid x → ∃ q, Quotes c f S (.eid x) q) :
    ∃ q, Quotes c f S n q := by
  cases n with
  | term t => exact ⟨_, .term t⟩
  | eid x => exact h x rfl

theorem quotes_of_iter : ∀ {k e}, iter (crefs c) S k e → ∃ q : QT L, Quotes c f S (.eid e) q
  | 0, _, h => h.elim
  | k + 1, e, h => by
    have hsub : ∀ n : Node T E, (∀ y ∈ n.refs, y ∈ crefs c e) →
        ∃ q, Quotes c f S n q := fun n hn =>
      quotes_node fun x hx => quotes_of_iter (h.2 x (hn x (by simp [hx, Node.refs])))
    obtain ⟨qs, hs⟩ := hsub (c e).s fun y hy => List.mem_append_left _ hy
    obtain ⟨qo, ho⟩ := hsub (c e).o fun y hy => List.mem_append_right _ hy
    exact ⟨_, .eid h.1 hs ho⟩

/-- On a closed, acyclic, finite `S` the quoting map is total. -/
theorem quotes_total (hfin : Covered S) (hC : Closed (crefs c) S) (hA : Acyclic (crefs c) S)
    {e : E} (he : S e) : ∃ q : QT L, Quotes c f S (.eid e) q :=
  let ⟨_, hk⟩ := ((K_eq_self_iff hfin).2 ⟨hC, hA⟩ e).2 he
  quotes_of_iter hk

theorem quotes_leaf_inv {n : Node T E} {l : L} (h : Quotes c f S n (.leaf l)) :
    ∃ t, n = .term t ∧ l = f t := by
  generalize hq : QT.leaf l = q at h
  cases h with
  | term t => injection hq with h; exact ⟨t, rfl, h⟩
  | eid => nomatch hq

theorem quotes_triple_inv {n : Node T E} {a o : QT L} {b : L}
    (h : Quotes c f S n (.triple a b o)) :
    ∃ e, n = .eid e ∧ S e ∧ Quotes c f S (c e).s a ∧ b = f (c e).p ∧ Quotes c f S (c e).o o := by
  generalize hq : QT.triple a b o = q at h
  cases h with
  | term => nomatch hq
  | eid he hs ho => injection hq with h1 h2 h3; subst h1 h2 h3; exact ⟨_, rfl, he, hs, rfl, ho⟩

/-- With distinct contents, two nodes with the same quote are equal (terms and eids). -/
theorem quotes_inj (hf : ∀ a b, f a = f b → a = b)
    (hd : ∀ x y, S x → S y → c x = c y → x = y) {n n' : Node T E} {q : QT L}
    (h : Quotes c f S n q) (h' : Quotes c f S n' q) : n = n' := by
  induction h generalizing n' with
  | term t =>
    obtain ⟨t', rfl, hl⟩ := quotes_leaf_inv h'
    rw [hf _ _ hl]
  | @eid e qs qo he _ _ ihs iho =>
    obtain ⟨e', rfl, he', hs', hp', ho'⟩ := quotes_triple_inv h'
    have hs := ihs hs'; have ho := iho ho'
    have hp := hf _ _ hp'
    have : c e = c e' := by
      cases hce : c e; cases hce' : c e'; simp_all
    rw [hd e e' he he' this]

/-- **`prop:queries` (ii).** For closed, acyclic, finite `S`, `q` is injective on the eids
of `S` exactly when no two records of `S` share `(s, p, o)`. -/
theorem quotes_injective_iff (hf : ∀ a b, f a = f b → a = b) (hfin : Covered S)
    (hC : Closed (crefs c) S) (hA : Acyclic (crefs c) S) :
    (∀ x y (q : QT L), Quotes c f S (.eid x) q → Quotes c f S (.eid y) q → x = y) ↔
      (∀ x y, S x → S y → c x = c y → x = y) := by
  constructor
  · intro hq x y hx hy hxy
    obtain ⟨q, hqx⟩ := quotes_total (f := f) hfin hC hA hx
    have hqy : Quotes c f S (.eid y) q := by
      cases hqx with
      | eid _ hs ho => rw [hxy] at hs ho ⊢; exact .eid hy hs ho
    exact hq x y q hqx hqy
  · intro hd x y q hx hy
    exact Node.eid.inj (quotes_inj hf hd hx hy)

/-! ## `prop:queries` (iii): basic graph patterns -/

/-- A pattern position: a variable or a constant. -/
inductive Pos (V T E : Type) where
  | var (v : V)
  | const (n : Node T E)

/-- An occurrence pattern `(s, p, o) ~ e`. -/
structure Pat (V T E : Type) where
  s : Pos V T E
  p : Pos V T E
  o : Pos V T E
  e : Option V

inductive QPos (V L : Type) where
  | var (v : V)
  | const (q : QT L)

structure QPat (V L : Type) where
  s : QPos V L
  p : QPos V L
  o : QPos V L
  e : Option V

def Pos.val (μ : V → Node T E) : Pos V T E → Node T E
  | .var v => μ v
  | .const n => n

def QPos.val (ν : V → QT L) : QPos V L → QT L
  | .var v => ν v
  | .const q => q

def Pos.Has (v : V) : Pos V T E → Prop
  | .var w => w = v
  | .const _ => False

/-- `v` occurs in the pattern. -/
def Pat.Has (P : Pat V T E) (v : V) : Prop :=
  P.s.Has v ∨ P.p.Has v ∨ P.o.Has v ∨ P.e = some v

/-- A record of `A` matches `P` under `μ`. -/
def Matches (c : E → Content T E) (A : E → Prop) (μ : V → Node T E) (P : Pat V T E) : Prop :=
  ∃ r, A r ∧ P.s.val μ = (c r).s ∧ P.p.val μ = .term (c r).p ∧ P.o.val μ = (c r).o ∧
    ∀ v, P.e = some v → μ v = .eid r

/-- `prop:queries` (i), taken as definition: the asserted RDF-star graph `G_A`. -/
def GA (c : E → Content T E) (f : T → L) (S A : E → Prop) (a : QT L) (b : L) (o : QT L) : Prop :=
  ∃ r, A r ∧ Quotes c f S (c r).s a ∧ b = f (c r).p ∧ Quotes c f S (c r).o o

/-- A triple of `G_A` matches the translated pattern under `ν`. -/
def QMatches (c : E → Content T E) (f : T → L) (S A : E → Prop) (ν : V → QT L)
    (P : QPat V L) : Prop :=
  ∃ a b o, GA c f S A a b o ∧ P.s.val ν = a ∧ P.p.val ν = .leaf b ∧ P.o.val ν = o ∧
    ∀ v, P.e = some v → ν v = .triple a b o

section
variable (c : E → Content T E) (f : T → L) (S : E → Prop)

/-- `q(P)` position by position: variables stay, constants are quoted. -/
inductive PosRel : Pos V T E → QPos V L → Prop
  | var (v : V) : PosRel (.var v) (.var v)
  | const {n : Node T E} {q : QT L} : Quotes c f S n q → PosRel (.const n) (.const q)

def PatRel (P : Pat V T E) (P' : QPat V L) : Prop :=
  PosRel c f S P.s P'.s ∧ PosRel c f S P.p P'.p ∧ PosRel c f S P.o P'.o ∧ P.e = P'.e

end

theorem posRel_val {p : Pos V T E} {p' : QPos V L} (h : PosRel c f S p p')
    {μ : V → Node T E} {ν : V → QT L} (hq : ∀ v, p.Has v → Quotes c f S (μ v) (ν v)) :
    Quotes c f S (p.val μ) (p'.val ν) := by
  cases h with
  | var v => exact hq v rfl
  | const h => exact h

theorem quotes_term {t : T} {q : QT L} (h : Quotes c f S (.term t) q) : q = .leaf (f t) :=
  quotes_functional h (.term t)

/-- **`prop:queries` (iii), forward.** `μ ↦ q ∘ μ` maps solutions of a pattern over `A`
into solutions of `q(P)` over `G_A`. -/
theorem matches_forward {A : E → Prop} (hAS : ∀ r, A r → S r) {P : Pat V T E} {P' : QPat V L}
    (hP : PatRel c f S P P') {μ : V → Node T E} {ν : V → QT L}
    (hq : ∀ v, P.Has v → Quotes c f S (μ v) (ν v)) (hm : Matches c A μ P) :
    QMatches c f S A ν P' := by
  obtain ⟨hs, hp, ho, he⟩ := hP
  obtain ⟨r, hr, ms, mp, mo, me⟩ := hm
  have qs := posRel_val hs fun v h => hq v (.inl h)
  have qp := posRel_val hp fun v h => hq v (.inr (.inl h))
  have qo := posRel_val ho fun v h => hq v (.inr (.inr (.inl h)))
  rw [ms] at qs; rw [mp] at qp; rw [mo] at qo
  refine ⟨_, _, _, ⟨r, hr, qs, rfl, qo⟩, rfl, quotes_term qp, rfl, fun v hv => ?_⟩
  have hqv := hq v (.inr (.inr (.inr (he ▸ hv))))
  rw [me v (he ▸ hv)] at hqv
  have := quotes_functional hqv (.eid (hAS r hr) qs qo)
  exact this

/-- **`prop:queries` (iii), backward.** With distinct contents on `S`, a solution of
`q(P)` over `G_A` pulls back along `q` to a solution of `P` over `A`. -/
theorem matches_backward (hf : ∀ a b, f a = f b → a = b)
    (hd : ∀ x y, S x → S y → c x = c y → x = y) {A : E → Prop} (hAS : ∀ r, A r → S r)
    {P : Pat V T E} {P' : QPat V L} (hP : PatRel c f S P P') {μ : V → Node T E}
    {ν : V → QT L} (hq : ∀ v, P.Has v → Quotes c f S (μ v) (ν v))
    (hm : QMatches c f S A ν P') : Matches c A μ P := by
  obtain ⟨hs, hp, ho, he⟩ := hP
  obtain ⟨a, b, o, ⟨r, hr, ras, rfl, rao⟩, ms, mp, mo, me⟩ := hm
  have qs := posRel_val hs fun v h => hq v (.inl h)
  have qp := posRel_val hp fun v h => hq v (.inr (.inl h))
  have qo := posRel_val ho fun v h => hq v (.inr (.inr (.inl h)))
  rw [ms] at qs; rw [mp] at qp; rw [mo] at qo
  refine ⟨r, hr, quotes_inj hf hd qs ras, quotes_inj hf hd qp (.term _),
    quotes_inj hf hd qo rao, fun v hv => ?_⟩
  have hqv := hq v (.inr (.inr (.inr hv)))
  rw [me v (he ▸ hv)] at hqv
  exact quotes_inj hf hd hqv (.eid (hAS r hr) ras rao)

/-- Every value a solution of `q(P)` gives a pattern variable is a quote. -/
theorem matches_values {A : E → Prop} {P : Pat V T E} {P' : QPat V L}
    (hP : PatRel c f S P P') {ν : V → QT L} (hm : QMatches c f S A ν P') (hAS : ∀ r, A r → S r)
    {v : V} (hv : P.Has v) : ∃ n, Quotes c f S n (ν v) := by
  obtain ⟨ps, pp, po, pe⟩ := P
  obtain ⟨ps', pp', po', pe'⟩ := P'
  obtain ⟨hs, hp, ho, he⟩ := hP
  obtain ⟨a, b, o, ⟨r, hr, ras, rfl, rao⟩, ms, mp, mo, me⟩ := hm
  simp only [Pat.Has] at hv he hs hp ho
  simp only at ms mp mo me
  rcases hv with h | h | h | h
  · cases hs with
    | var w => cases h; exact ⟨_, ms ▸ ras⟩
    | const => exact h.elim
  · cases hp with
    | var w => cases h; exact ⟨_, mp ▸ Quotes.term _⟩
    | const => exact h.elim
  · cases ho with
    | var w => cases h; exact ⟨_, mo ▸ rao⟩
    | const => exact h.elim
  · rw [me v (he ▸ h)]; exact ⟨_, Quotes.eid (hAS r hr) ras rao⟩

/-- Pointwise relation between two lists. -/
inductive Forall2 {α β : Type} (R : α → β → Prop) : List α → List β → Prop
  | nil : Forall2 R [] []
  | cons {a b l l'} : R a b → Forall2 R l l' → Forall2 R (a :: l) (b :: l')

/-- Solutions of a basic graph pattern (a list of occurrence patterns). -/
def Sol (c : E → Content T E) (A : E → Prop) (μ : V → Node T E) (B : List (Pat V T E)) : Prop :=
  ∀ P ∈ B, Matches c A μ P

def QSol (c : E → Content T E) (f : T → L) (S A : E → Prop) (ν : V → QT L)
    (B : List (QPat V L)) : Prop :=
  ∀ P ∈ B, QMatches c f S A ν P

/-- **`prop:queries` (iii).** For a BGP `B` with translation `B'`, and contents distinct
on `S`: `μ` solves `B` over `A` iff `q ∘ μ` solves `B'` over `G_A`; every solution of `B'`
is of this form; and `μ` is determined on the pattern variables by `q ∘ μ`. -/
theorem bgp_view [Inhabited T] (hf : ∀ a b, f a = f b → a = b)
    (hd : ∀ x y, S x → S y → c x = c y → x = y) {A : E → Prop} (hAS : ∀ r, A r → S r)
    {B : List (Pat V T E)} {B' : List (QPat V L)}
    (hB : Forall2 (PatRel c f S) B B') :
    (∀ (μ : V → Node T E) (ν : V → QT L),
      (∀ v, (∃ P ∈ B, P.Has v) → Quotes c f S (μ v) (ν v)) →
        (Sol c A μ B ↔ QSol c f S A ν B')) ∧
    (∀ ν : V → QT L, QSol c f S A ν B' →
      ∃ μ : V → Node T E, ∀ v, (∃ P ∈ B, P.Has v) → Quotes c f S (μ v) (ν v)) ∧
    (∀ (μ μ' : V → Node T E) (ν : V → QT L),
      (∀ v, (∃ P ∈ B, P.Has v) → Quotes c f S (μ v) (ν v)) →
      (∀ v, (∃ P ∈ B, P.Has v) → Quotes c f S (μ' v) (ν v)) →
        ∀ v, (∃ P ∈ B, P.Has v) → μ v = μ' v) := by
  classical
  refine ⟨fun μ ν hq => ?_, fun ν hsol => ?_, fun μ μ' ν h h' v hv =>
    quotes_inj hf hd (h v hv) (h' v hv)⟩
  · induction hB with
    | nil => exact ⟨fun _ _ h => (nomatch h), fun _ _ h => (nomatch h)⟩
    | @cons P P' B B' hPP' _ ih =>
      have hqP : ∀ v, P.Has v → Quotes c f S (μ v) (ν v) := fun v h =>
        hq v ⟨P, List.mem_cons_self .., h⟩
      have ih := ih fun v ⟨Q, hQ, h⟩ => hq v ⟨Q, List.mem_cons_of_mem _ hQ, h⟩
      constructor
      · intro hs Q' hQ'
        rcases List.mem_cons.1 hQ' with rfl | hQ'
        · exact matches_forward hAS hPP' hqP (hs P (List.mem_cons_self ..))
        · exact ih.1 (fun Q hQ => hs Q (List.mem_cons_of_mem _ hQ)) Q' hQ'
      · intro hs Q hQ
        rcases List.mem_cons.1 hQ with rfl | hQ
        · exact matches_backward hf hd hAS hPP' hqP (hs _ (List.mem_cons_self ..))
        · exact ih.2 (fun Q' hQ' => hs Q' (List.mem_cons_of_mem _ hQ')) Q hQ
  · -- pick a preimage for every pattern variable
    have hval : ∀ v, (∃ P ∈ B, P.Has v) → ∃ n, Quotes c f S n (ν v) := by
      intro v ⟨P, hP, hv⟩
      obtain ⟨P', hP', hrel⟩ : ∃ P' ∈ B', PatRel c f S P P' := by
        clear hsol
        induction hB with
        | nil => nomatch hP
        | cons h _ ih =>
          rcases List.mem_cons.1 hP with rfl | hP
          · exact ⟨_, List.mem_cons_self .., h⟩
          · obtain ⟨P', hP', hr⟩ := ih hP; exact ⟨P', List.mem_cons_of_mem _ hP', hr⟩
      exact matches_values hrel (hsol P' hP') hAS hv
    refine ⟨fun v => if h : ∃ n, Quotes c f S n (ν v) then Classical.choose h
      else .term default, fun v hv => ?_⟩
    have h := hval v hv
    simp only [h, dite_true]
    exact Classical.choose_spec h

end StatementExport
