import StatementExport.Export

/-!
# OneGraph encodings

The temporal encoding `T_f(S, 𝒜)` is a set of target statements with identifiers that
are fresh by construction: one per record, one per present clock, one membership per
time statement and one membership per asserted record. This file proves that the
encoding is lossless (Theorem `thm:temporal`: it determines the store, the asserted set
and all eight fields of every record) and the structural image conditions it rests on:
every record vertex has an incoming edge, memberships are never targets, only their
memberships target time statements, and the encoding is acyclic when `S` is closed and
acyclic. It also proves Proposition `prop:rdf12` for the RDF 1.2 reifier encoding.
Typing of positions (IRI, literal, blank node) and the RDF-star non-isomorphism condition
are not modelled here; the latter is the quoting map of `Quoting.lean`.
-/

namespace StatementExport

open Relation

variable {T E L : Type}

inductive Clock where
  | a | d | v1 | v2
  deriving DecidableEq

/-- A record: content, transaction start `a`, optional end `d`, valid interval
`[v₁, v₂)` with `none` for `-∞` and `+∞`. -/
structure TRec (T E : Type) where
  content : Content T E
  a : Nat
  d : Option Nat
  v1 : Option Int
  v2 : Option Int

def TRec.clock (r : TRec T E) : Clock → Option Int
  | .a => some r.a
  | .d => r.d.map Int.ofNat
  | .v1 => r.v1
  | .v2 => r.v2

theorem Content.ext' {x y : Content T E} (hs : x.s = y.s) (hp : x.p = y.p) (ho : x.o = y.o) :
    x = y := by
  cases x; cases y; simp_all

theorem TRec.ext_clock {x y : TRec T E} (hc : x.content = y.content)
    (hk : ∀ k, x.clock k = y.clock k) : x = y := by
  have ha := hk .a; have hd := hk .d; have h1 := hk .v1; have h2 := hk .v2
  simp only [TRec.clock, Option.some.injEq, Int.ofNat_inj] at ha hd h1 h2
  have hd' : x.d = y.d := by
    cases hx : x.d <;> cases hy : y.d <;> simp_all <;> omega
  cases x; cases y; simp_all

/-! ## Temporal encoding -/

inductive VId (E : Type) where
  | record (e : E)
  | time (e : E) (k : Clock)
  | memRec (e : E)
  | memTime (e : E) (k : Clock)

inductive TTerm (E L : Type) where
  | vtx (v : VId E)
  | leaf (l : L)
  | int (z : Int)
  | graph

inductive Pred (L : Type) where
  | iri (l : L)
  | inn
  | tau (k : Clock)

structure TStmt (E L : Type) where
  id : VId E
  s : TTerm E L
  p : Pred L
  o : TTerm E L

def encT (f : T → L) : Node T E → TTerm E L
  | .term t => .leaf (f t)
  | .eid x => .vtx (.record x)

/-- `T_f(S, 𝒜)` as a set of target statements. -/
def Enc (R : E → TRec T E) (f : T → L) (S A : E → Prop) (st : TStmt E L) : Prop :=
  match st.id with
  | .record e => S e ∧ st.s = encT f (R e).content.s ∧ st.p = .iri (f (R e).content.p) ∧
      st.o = encT f (R e).content.o
  | .time e k => S e ∧ ∃ z, (R e).clock k = some z ∧ st.s = .vtx (.record e) ∧ st.p = .tau k ∧
      st.o = .int z
  | .memTime e k => S e ∧ (∃ z, (R e).clock k = some z) ∧ st.s = .vtx (.time e k) ∧
      st.p = .inn ∧ st.o = .graph
  | .memRec e => A e ∧ st.s = .vtx (.record e) ∧ st.p = .inn ∧ st.o = .graph

theorem encT_inj {f : T → L} (hf : ∀ a b, f a = f b → a = b) {n n' : Node T E}
    (h : (encT f n : TTerm E L) = encT f n') : n = n' := by
  cases n <;> cases n' <;> simp_all [encT]
  exact hf _ _ h

/-- **Theorem `thm:temporal` (lossless).** `T_f(S, 𝒜)` determines `S`, `𝒜` and all eight
fields of every record of `S`: `R^τ_f(T_f(S, 𝒜)) = (S, 𝒜)`. -/
theorem temporal_lossless {R R' : E → TRec T E} {f : T → L} (hf : ∀ a b, f a = f b → a = b)
    {S A S' A' : E → Prop} (h : ∀ st, Enc R f S A st ↔ Enc R' f S' A' st) :
    (∀ e, S e ↔ S' e) ∧ (∀ e, A e ↔ A' e) ∧ ∀ e, S e → R e = R' e := by
  have hS : ∀ e, S e ↔ S' e := fun e =>
    ⟨fun he => ((h ⟨.record e, _, _, _⟩).1 ⟨he, rfl, rfl, rfl⟩).1,
     fun he => ((h ⟨.record e, _, _, _⟩).2 ⟨he, rfl, rfl, rfl⟩).1⟩
  refine ⟨hS, fun e => ⟨fun he => ((h ⟨.memRec e, _, _, _⟩).1 ⟨he, rfl, rfl, rfl⟩).1,
    fun he => ((h ⟨.memRec e, _, _, _⟩).2 ⟨he, rfl, rfl, rfl⟩).1⟩, fun e he => ?_⟩
  obtain ⟨-, hs, hp, ho⟩ := (h ⟨.record e, _, _, _⟩).1 ⟨he, rfl, rfl, rfl⟩
  simp only [Pred.iri.injEq] at hs hp ho
  have hc : (R e).content = (R' e).content :=
    Content.ext' (encT_inj hf hs) (hf _ _ hp) (encT_inj hf ho)
  have clock_some : ∀ (R₁ R₂ : E → TRec T E) (S₁ A₁ S₂ A₂ : E → Prop),
      (∀ st, Enc R₁ f S₁ A₁ st → Enc R₂ f S₂ A₂ st) → S₁ e →
      ∀ k z, (R₁ e).clock k = some z → (R₂ e).clock k = some z := by
    intro R₁ R₂ S₁ A₁ S₂ A₂ himp he k z hz
    obtain ⟨-, z', hz', -, -, hzz⟩ := himp ⟨.time e k, .vtx (.record e), .tau k, .int z⟩
      ⟨he, z, hz, rfl, rfl, rfl⟩
    cases hzz; exact hz'
  refine TRec.ext_clock hc fun k => ?_
  cases h1 : (R e).clock k with
  | some z => exact (clock_some R R' S A S' A' (fun st => (h st).1) he k z h1).symm
  | none =>
    cases h2 : (R' e).clock k with
    | none => rfl
    | some z =>
      have := clock_some R' R S' A' S A (fun st => (h st).2) ((hS e).1 he) k z h2
      rw [h1] at this; nomatch this

/-- `x` has a role edge (subject or object) to `y` in the encoding. -/
def Points (R : E → TRec T E) (f : T → L) (S A : E → Prop) (x y : VId E) : Prop :=
  ∃ st : TStmt E L, Enc R f S A st ∧ st.id = x ∧ (st.s = .vtx y ∨ st.o = .vtx y)

theorem encT_vtx {f : T → L} {n : Node T E} {y : VId E} (h : (encT f n : TTerm E L) = .vtx y) :
    ∃ x, n = .eid x ∧ y = .record x := by
  cases n with
  | term => simp [encT] at h
  | eid x => simp only [encT, TTerm.vtx.injEq] at h; exact ⟨x, rfl, h.symm⟩

/-- Structure of the role edges of `T_f(S, 𝒜)`. -/
theorem points_cases {R : E → TRec T E} {f : T → L} {S A : E → Prop} {x y : VId E}
    (h : Points R f S A x y) :
    (∃ e e', x = .record e ∧ y = .record e' ∧ S e ∧ e' ∈ crefs (fun e => (R e).content) e) ∨
    (∃ e k, x = .time e k ∧ y = .record e) ∨
    (∃ e k, x = .memTime e k ∧ y = .time e k) ∨
    (∃ e, x = .memRec e ∧ y = .record e) := by
  obtain ⟨⟨id, s, p, o⟩, hst, rfl, hy⟩ := h
  cases id with
  | record e =>
    obtain ⟨he, hs, -, ho⟩ := hst
    simp only at hs ho hy
    refine .inl ?_
    rcases hy with hy | hy
    · obtain ⟨x, hx, rfl⟩ := encT_vtx (hs ▸ hy)
      exact ⟨e, x, rfl, rfl, he, mem_crefs.2 ⟨.s, hx⟩⟩
    · obtain ⟨x, hx, rfl⟩ := encT_vtx (ho ▸ hy)
      exact ⟨e, x, rfl, rfl, he, mem_crefs.2 ⟨.o, hx⟩⟩
  | time e k =>
    obtain ⟨-, z, -, hs, -, ho⟩ := hst
    simp only at hs ho hy
    rcases hy with hy | hy
    · rw [hs] at hy; cases hy; exact .inr (.inl ⟨e, k, rfl, rfl⟩)
    · rw [ho] at hy; nomatch hy
  | memTime e k =>
    obtain ⟨-, -, hs, -, ho⟩ := hst
    simp only at hs ho hy
    rcases hy with hy | hy
    · rw [hs] at hy; cases hy; exact .inr (.inr (.inl ⟨e, k, rfl, rfl⟩))
    · rw [ho] at hy; nomatch hy
  | memRec e =>
    obtain ⟨-, hs, -, ho⟩ := hst
    simp only at hs ho hy
    rcases hy with hy | hy
    · rw [hs] at hy; cases hy; exact .inr (.inr (.inr ⟨e, rfl, rfl⟩))
    · rw [ho] at hy; nomatch hy

/-- Every record vertex has an incoming edge, from its `τ_a` statement. -/
theorem record_has_incoming {R : E → TRec T E} {f : T → L} {S A : E → Prop} {e : E}
    (he : S e) : Points R f S A (.time e .a) (.record e) :=
  ⟨⟨.time e .a, .vtx (.record e), .tau .a, .int (R e).a⟩, ⟨he, _, rfl, rfl, rfl, rfl⟩, rfl, .inl rfl⟩

/-- Memberships are never targets; only its membership targets a time statement. -/
theorem membership_unreferenced {R : E → TRec T E} {f : T → L} {S A : E → Prop}
    {x y : VId E} (h : Points R f S A x y) :
    (∀ e, y ≠ .memRec e) ∧ (∀ e k, y ≠ .memTime e k) ∧
    (∀ e k, y = .time e k → x = .memTime e k) := by
  rcases points_cases h with ⟨_, _, _, rfl, _⟩ | ⟨_, _, _, rfl⟩ | ⟨_, _, rfl, rfl⟩ | ⟨_, _, rfl⟩ <;>
    refine ⟨fun _ h => (nomatch h), fun _ _ h => (nomatch h), ?_⟩ <;> intro _ _ h <;> cases h
  rfl

theorem transGen_first {α : Type} {r : α → α → Prop} {a b : α} (h : TransGen r a b) :
    ∃ c, r a c ∧ (c = b ∨ TransGen r c b) := by
  induction h with
  | single h => exact ⟨_, h, .inl rfl⟩
  | tail _ h ih =>
    obtain ⟨c, hac, hc⟩ := ih
    rcases hc with rfl | hc
    · exact ⟨_, hac, .inr (.single h)⟩
    · exact ⟨c, hac, .inr (.tail hc h)⟩

/-- Some vertex on a cycle points to each vertex of the cycle. -/
theorem cycle_pred {α : Type} {r : α → α → Prop} {a : α} (h : TransGen r a a) :
    ∃ c, r a c ∧ TransGen r c c := by
  obtain ⟨c, hac, hc⟩ := transGen_first h
  rcases hc with rfl | hc
  · exact ⟨_, hac, .single hac⟩
  · exact ⟨c, hac, .tail hc hac⟩

/-- **Theorem `thm:temporal` (acyclic).** For closed, acyclic `S` the role edges of
`T_f(S, 𝒜)` have no cycle. -/
theorem temporal_acyclic {R : E → TRec T E} {f : T → L} {S A : E → Prop}
    (hC : Closed (crefs fun e => (R e).content) S) (hAc : Acyclic (crefs fun e => (R e).content) S)
    (x : VId E) : ¬ TransGen (fun y x => Points R f S A x y) x x := by
  let c : E → Content T E := fun e => (R e).content
  have chain : ∀ a b, TransGen (fun y x => Points R f S A x y) a b → ∀ e, b = .record e →
      ∃ e', a = .record e' ∧ TransGen (Ref (crefs c) S) e' e := by
    intro a b hab
    induction hab with
    | single h =>
      intro e hb; subst hb
      rcases points_cases h with ⟨e₁, e', h1, rfl, he, hr⟩ | ⟨_, _, h1, _⟩ | ⟨_, _, h1, _⟩ |
          ⟨_, h1, _⟩ <;> cases h1
      exact ⟨e', rfl, .single ⟨he, hC _ he _ hr, hr⟩⟩
    | tail _ h ih =>
      intro e hb; subst hb
      rcases points_cases h with ⟨e₁, e', h1, rfl, he, hr⟩ | ⟨_, _, h1, _⟩ | ⟨_, _, h1, _⟩ |
          ⟨_, h1, _⟩ <;> cases h1
      obtain ⟨e'', rfl, hch⟩ := ih e' rfl
      exact ⟨e'', rfl, .tail hch ⟨he, hC _ he _ hr, hr⟩⟩
  intro hx
  obtain ⟨c₁, hpt, hc₁⟩ := cycle_pred hx
  obtain ⟨hmr, hmt, htime⟩ := membership_unreferenced hpt
  cases x with
  | record e =>
    obtain ⟨e', he', hch⟩ := chain _ _ hx e rfl
    cases he'; exact hAc _ hch
  | time e k =>
    have := htime e k rfl; subst this
    obtain ⟨_, hpt', -⟩ := cycle_pred hc₁
    exact (membership_unreferenced hpt').2.1 e k rfl
  | memRec e => exact hmr e rfl
  | memTime e k => exact hmt e k rfl

/-! ## RDF 1.2 reifier encoding -/

inductive RTerm (E L : Type) where
  | name (e : E)
  | leaf (l : L)
  | int (z : Int)
  | tt (s : RTerm E L) (p : L) (o : RTerm E L)

inductive RPred (L : Type) where
  | reifies
  | iri (l : L)
  | tau (k : Clock)

structure RTriple (E L : Type) where
  s : RTerm E L
  p : RPred L
  o : RTerm E L

/-- `g`: `ι` on eids, `f` on terms. -/
def gNode (f : T → L) : Node T E → RTerm E L
  | .term t => .leaf (f t)
  | .eid x => .name x

/-- `Ω_f(S, 𝒜)`: reifying triples, time triples and asserted triples. -/
def Omega (R : E → TRec T E) (f : T → L) (S A : E → Prop) (tr : RTriple E L) : Prop :=
  (∃ e, S e ∧ tr = ⟨.name e, .reifies,
      .tt (gNode f (R e).content.s) (f (R e).content.p) (gNode f (R e).content.o)⟩) ∨
  (∃ e k z, S e ∧ (R e).clock k = some z ∧ tr = ⟨.name e, .tau k, .int z⟩) ∨
  (∃ e, A e ∧ tr = ⟨gNode f (R e).content.s, .iri (f (R e).content.p), gNode f (R e).content.o⟩)

/-- Reifying and time triples. -/
def RTriple.Reif (tr : RTriple E L) : Prop := tr.p = .reifies ∨ ∃ k, tr.p = .tau k

theorem gNode_inj {f : T → L} (hf : ∀ a b, f a = f b → a = b) {n n' : Node T E}
    (h : (gNode f n : RTerm E L) = gNode f n') : n = n' := by
  cases n <;> cases n' <;> simp_all [gNode]
  exact hf _ _ h

/-- **Proposition `prop:rdf12` (first part).** For every `S`, closed or not and acyclic or
not, the reifying and time triples of `Ω_f(S, 𝒜)` determine `S` with all eight fields. -/
theorem rdf12_lossless {R R' : E → TRec T E} {f : T → L} (hf : ∀ a b, f a = f b → a = b)
    {S A S' A' : E → Prop}
    (h : ∀ tr : RTriple E L, tr.Reif → (Omega R f S A tr ↔ Omega R' f S' A' tr)) :
    (∀ e, S e ↔ S' e) ∧ ∀ e, S e → R e = R' e := by
  -- the reifying triple of `e` in one encoding is a reifying triple of `e` in the other
  have reif : ∀ (R₁ R₂ : E → TRec T E) (S₁ A₁ S₂ A₂ : E → Prop),
      (∀ tr : RTriple E L, tr.Reif → Omega R₁ f S₁ A₁ tr → Omega R₂ f S₂ A₂ tr) →
      ∀ e, S₁ e → S₂ e ∧ (R₁ e).content = (R₂ e).content := by
    intro R₁ R₂ S₁ A₁ S₂ A₂ himp e he
    rcases himp _ (.inl rfl) (.inl ⟨e, he, rfl⟩) with ⟨e', he', heq⟩ | ⟨_, _, _, _, _, heq⟩ |
        ⟨_, _, heq⟩
    · simp only [RTriple.mk.injEq, RTerm.name.injEq, RTerm.tt.injEq, true_and] at heq
      obtain ⟨rfl, hs, hp, ho⟩ := heq
      exact ⟨he', Content.ext' (gNode_inj hf hs) (hf _ _ hp) (gNode_inj hf ho)⟩
    · simp at heq
    · simp at heq
  have clock : ∀ (R₁ R₂ : E → TRec T E) (S₁ A₁ S₂ A₂ : E → Prop),
      (∀ tr : RTriple E L, tr.Reif → Omega R₁ f S₁ A₁ tr → Omega R₂ f S₂ A₂ tr) →
      ∀ e k z, S₁ e → (R₁ e).clock k = some z → (R₂ e).clock k = some z := by
    intro R₁ R₂ S₁ A₁ S₂ A₂ himp e k z he hz
    rcases himp _ (.inr ⟨k, rfl⟩) (.inr (.inl ⟨e, k, z, he, hz, rfl⟩)) with ⟨_, _, heq⟩ |
        ⟨e', k', z', -, hz', heq⟩ | ⟨_, _, heq⟩
    · simp at heq
    · simp only [RTriple.mk.injEq, RTerm.name.injEq, RPred.tau.injEq, RTerm.int.injEq] at heq
      obtain ⟨rfl, rfl, rfl⟩ := heq; exact hz'
    · simp at heq
  have h₁ := fun tr hr => (h tr hr).1
  have h₂ := fun tr hr => (h tr hr).2
  refine ⟨fun e => ⟨fun he => (reif R R' S A S' A' h₁ e he).1,
    fun he => (reif R' R S' A' S A h₂ e he).1⟩, fun e he => ?_⟩
  refine TRec.ext_clock (reif R R' S A S' A' h₁ e he).2 fun k => ?_
  have he' := (reif R R' S A S' A' h₁ e he).1
  cases h1 : (R e).clock k with
  | some z => exact (clock R R' S A S' A' h₁ e k z he h1).symm
  | none =>
    cases h2 : (R' e).clock k with
    | none => rfl
    | some z => have := clock R' R S' A' S A h₂ e k z he' h2; rw [h1] at this; nomatch this

/-- **Proposition `prop:rdf12` (second part).** Reading a record as asserted when its
content triple occurs recovers `𝒜` exactly when no record of `𝒜` shares its content with
a record of `S ∖ 𝒜`. -/
theorem rdf12_asserted {R : E → TRec T E} {f : T → L} (hf : ∀ a b, f a = f b → a = b)
    {S A : E → Prop} :
    (∀ r, S r → (Omega R f S A ⟨gNode f (R r).content.s, .iri (f (R r).content.p),
        gNode f (R r).content.o⟩ ↔ A r)) ↔
      ¬ ∃ r r', A r ∧ S r' ∧ ¬ A r' ∧ (R r).content = (R r').content := by
  classical
  have asserted : ∀ r, Omega R f S A ⟨gNode f (R r).content.s, .iri (f (R r).content.p),
      gNode f (R r).content.o⟩ ↔ ∃ r'', A r'' ∧ (R r'').content = (R r).content := by
    intro r
    constructor
    · rintro (⟨_, _, heq⟩ | ⟨_, _, _, _, _, heq⟩ | ⟨r'', hr'', heq⟩)
      · simp at heq
      · simp at heq
      · simp only [RTriple.mk.injEq, RPred.iri.injEq] at heq
        obtain ⟨hs, hp, ho⟩ := heq
        exact ⟨r'', hr'', Content.ext' (gNode_inj hf hs.symm) (hf _ _ hp.symm)
          (gNode_inj hf ho.symm)⟩
    · rintro ⟨r'', hr'', heq⟩
      exact .inr (.inr ⟨r'', hr'', by rw [heq]⟩)
  simp only [asserted]
  constructor
  · rintro h ⟨r, r', hr, hr', hnr', heq⟩
    exact hnr' ((h r' hr').1 ⟨r, hr, heq⟩)
  · intro h r _
    refine ⟨fun ⟨r'', hr'', heq⟩ => Classical.byContradiction fun hnr => ?_,
      fun hr => ⟨r, hr, rfl⟩⟩
    exact h ⟨r'', r, hr'', ‹S r›, hnr, heq⟩

end StatementExport
