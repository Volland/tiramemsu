/-!
# The export kernel

Records are identified by their eids `E`; `refs r` lists the eid endpoints of the
record `r`. Record sets are predicates `E → Prop`; finiteness is an explicit list
cover. This file proves `thm:kernel` (greatest direct-export subset), `cor:obstructions`,
`prop:dual` (reference closure) and the kernel part of `prop:bit` (exportable bit),
generically in `refs`.
-/

namespace StatementExport

open Relation

variable {E : Type}

/-- A list covers `U`: `U` is finite. -/
def Covered (U : E → Prop) : Prop := ∃ L : List E, ∀ r, U r → r ∈ L

section
variable (refs : E → List E)

/-- `U` is closed: every reference of a member is a member. -/
def Closed (U : E → Prop) : Prop := ∀ r, U r → ∀ x ∈ refs r, U x

/-- `Ref U y x`: inside `U`, the record `x` references `y`. -/
def Ref (U : E → Prop) (y x : E) : Prop := U x ∧ U y ∧ y ∈ refs x

/-- No directed reference cycle inside `U`. -/
def Acyclic (U : E → Prop) : Prop := ∀ a, ¬ TransGen (Ref refs U) a a

/-- One step of reachability from a member of `T` to one of its references. -/
def Step (T : E → Prop) (x y : E) : Prop := T x ∧ y ∈ refs x

/-- Reflexive-transitive reachability along `Step T`. -/
inductive Reach (T : E → Prop) : E → E → Prop
  | refl (a : E) : Reach T a a
  | head {a b c : E} : Step refs T a b → Reach T b c → Reach T a c

/-- `F_S(X) = {r ∈ S : refs(r) ⊆ X}`. -/
def F (S X : E → Prop) : E → Prop := fun r => S r ∧ ∀ x ∈ refs r, X x

/-- `F_S^k(∅)`. -/
def iter (S : E → Prop) : Nat → E → Prop
  | 0 => fun _ => False
  | k + 1 => F refs S (iter S k)

/-- The export kernel `K(S) = ⋃ₖ F_S^k(∅)`. -/
def K (S : E → Prop) : E → Prop := fun r => ∃ k, iter refs S k r

end

variable {refs : E → List E}

theorem Reach.trans {T : E → Prop} {a b c : E} (h₁ : Reach refs T a b) (h₂ : Reach refs T b c) :
    Reach refs T a c := by
  induction h₁ with
  | refl => exact h₂
  | head s _ ih => exact .head s (ih h₂)

theorem Reach.tail {T : E → Prop} {a b c : E} (h : Reach refs T a b) (s : Step refs T b c) :
    Reach refs T a c := h.trans (.head s (.refl c))

theorem TransGen.ref_mono {U V : E → Prop} (hUV : ∀ r, U r → V r) {a b : E}
    (h : TransGen (Ref refs U) a b) : TransGen (Ref refs V) a b := by
  induction h with
  | single h => exact .single ⟨hUV _ h.1, hUV _ h.2.1, h.2.2⟩
  | tail _ h ih => exact .tail ih ⟨hUV _ h.1, hUV _ h.2.1, h.2.2⟩

theorem acc_irrefl {α : Sort _} {r : α → α → Prop} {a : α} (h : Acc r a) : ¬ r a a := by
  induction h with
  | intro x _ ih => exact fun hx => ih x hx hx

/-! ## Basic properties of the iteration -/

theorem iter_succ {S : E → Prop} : ∀ {k r}, iter refs S k r → iter refs S (k + 1) r
  | 0, _, h => h.elim
  | _ + 1, _, h => ⟨h.1, fun x hx => iter_succ (h.2 x hx)⟩

theorem iter_mono {S : E → Prop} {k k' : Nat} (hk : k ≤ k') {r : E} (h : iter refs S k r) :
    iter refs S k' r := by
  induction hk with
  | refl => exact h
  | step _ ih => exact iter_succ ih

theorem iter_sub {S : E → Prop} : ∀ {k r}, iter refs S k r → S r
  | 0, _, h => h.elim
  | _ + 1, _, h => h.1

theorem K_sub {S : E → Prop} {r : E} (h : K refs S r) : S r :=
  let ⟨_, hk⟩ := h; iter_sub hk

theorem iter_set_mono {S S' : E → Prop} (h : ∀ r, S r → S' r) :
    ∀ {k r}, iter refs S k r → iter refs S' k r
  | 0, _, hr => hr.elim
  | _ + 1, _, hr => ⟨h _ hr.1, fun x hx => iter_set_mono h (hr.2 x hx)⟩

/-- Monotonicity: `S ⊆ S'` implies `K(S) ⊆ K(S')`. -/
theorem K_mono {S S' : E → Prop} (h : ∀ r, S r → S' r) {r : E} (hr : K refs S r) :
    K refs S' r :=
  let ⟨k, hk⟩ := hr; ⟨k, iter_set_mono h hk⟩

theorem common_level {S : E → Prop} :
    ∀ l : List E, (∀ x ∈ l, K refs S x) → ∃ k, ∀ x ∈ l, iter refs S k x
  | [], _ => ⟨0, fun _ hx => nomatch hx⟩
  | y :: l, h => by
    obtain ⟨k₁, hk₁⟩ := h y (List.mem_cons_self ..)
    obtain ⟨k₂, hk₂⟩ := common_level l (fun x hx => h x (List.mem_cons_of_mem _ hx))
    refine ⟨max k₁ k₂, fun x hx => ?_⟩
    cases hx with
    | head => exact iter_mono (Nat.le_max_left ..) hk₁
    | tail _ hx => exact iter_mono (Nat.le_max_right ..) (hk₂ x hx)

/-- `K(S)` is a fixpoint of `F_S`. -/
theorem K_fix {S : E → Prop} {r : E} : K refs S r ↔ S r ∧ ∀ x ∈ refs r, K refs S x := by
  constructor
  · rintro ⟨k, hk⟩
    cases k with
    | zero => exact hk.elim
    | succ k => exact ⟨hk.1, fun x hx => ⟨k, hk.2 x hx⟩⟩
  · rintro ⟨hS, hx⟩
    obtain ⟨k, hk⟩ := common_level (refs r) hx
    exact ⟨k + 1, hS, hk⟩

/-! ## `thm:kernel`: closed, acyclic, greatest -/

theorem K_closed (S : E → Prop) : Closed refs (K refs S) :=
  fun _ hr => (K_fix.1 hr).2

theorem acc_of_iter {S : E → Prop} : ∀ {k r}, iter refs S k r → Acc (Ref refs (K refs S)) r
  | 0, _, h => h.elim
  | _ + 1, _, h => ⟨_, fun y hy => acc_of_iter (h.2 y hy.2.2)⟩

theorem K_acyclic (S : E → Prop) : Acyclic refs (K refs S) := by
  intro a h
  have ha : K refs S a := by
    cases h with
    | single h => exact h.1
    | tail _ h => exact h.1
  obtain ⟨k, hk⟩ := ha
  exact acc_irrefl (acc_of_iter hk).transGen h

/-- Pigeonhole: `n + 1` values in a list of length `n` repeat. -/
theorem pigeonhole {α : Type} :
    ∀ (L : List α) (p : Nat → α), (∀ i, i ≤ L.length → p i ∈ L) →
      ∃ i j, i < j ∧ j ≤ L.length ∧ p i = p j
  | [], p, h => nomatch h 0 (Nat.le_refl _)
  | a :: L, p, h => by
    classical
    by_cases hk : ∃ k, k ≤ L.length + 1 ∧ p k = a
    · obtain ⟨k, hkle, hka⟩ := hk
      by_cases hk' : ∃ j, j ≤ L.length + 1 ∧ j ≠ k ∧ p j = a
      · obtain ⟨j, hjle, hjk, hja⟩ := hk'
        rcases Nat.lt_or_gt_of_ne hjk with hlt | hlt
        · exact ⟨j, k, hlt, hkle, hja.trans hka.symm⟩
        · exact ⟨k, j, hlt, hjle, hka.trans hja.symm⟩
      · let g : Nat → Nat := fun i => if i < k then i else i + 1
        have hg : ∀ i j, i < j → g i < g j := by
          intro i j hij; simp only [g]; split <;> split <;> omega
        have hgk : ∀ i, g i ≠ k := by intro i; simp only [g]; split <;> omega
        have hmem : ∀ i, i ≤ L.length → p (g i) ∈ L := by
          intro i hi
          have hgi : g i ≤ L.length + 1 := by simp only [g]; split <;> omega
          have := h (g i) (by simpa using hgi)
          cases List.mem_cons.1 this with
          | inl he => exact absurd ⟨g i, hgi, hgk i, he⟩ hk'
          | inr hm => exact hm
        obtain ⟨i, j, hij, hjle, hp⟩ := pigeonhole L (fun i => p (g i)) hmem
        refine ⟨g i, g j, hg i j hij, ?_, hp⟩
        simp only [List.length_cons, g]; split <;> omega
    · have hmem : ∀ i, i ≤ L.length → p i ∈ L := by
        intro i hi
        cases List.mem_cons.1 (h i (by simp; omega)) with
        | inl he => exact absurd ⟨i, by omega, he⟩ hk
        | inr hm => exact hm
      obtain ⟨i, j, hij, hjle, hp⟩ := pigeonhole L p hmem
      exact ⟨i, j, hij, by simp; omega, hp⟩

/-- A member of a closed `U ⊆ S` outside the kernel references another such member. -/
theorem escape {S U : E → Prop} (hUS : ∀ r, U r → S r) (hU : Closed refs U) {r : E}
    (hr : U r) (hK : ¬ K refs S r) : ∃ y ∈ refs r, U y ∧ ¬ K refs S y := by
  classical
  refine Classical.byContradiction fun hno => hK (K_fix.2 ⟨hUS r hr, fun x hx => ?_⟩)
  exact Classical.byContradiction fun hx' => hno ⟨x, hx, hU r hr x hx, hx'⟩

/-- **`thm:kernel` (greatest).** Every finite, closed, acyclic `U ⊆ S` lies in `K(S)`. -/
theorem K_greatest {S U : E → Prop} (hfin : Covered U) (hUS : ∀ r, U r → S r)
    (hU : Closed refs U) (hA : Acyclic refs U) : ∀ r, U r → K refs S r := by
  classical
  intro r hr
  refine Classical.byContradiction fun hK => ?_
  obtain ⟨L, hL⟩ := hfin
  let P : E → Prop := fun x => U x ∧ ¬ K refs S x
  have next : ∀ x : {x // P x}, ∃ y : {y // P y}, y.1 ∈ refs x.1 := fun x =>
    let ⟨y, hy, hUy, hKy⟩ := escape hUS hU x.2.1 x.2.2
    ⟨⟨y, hUy, hKy⟩, hy⟩
  let p : Nat → {x // P x} := fun n =>
    Nat.rec (motive := fun _ => {x // P x}) ⟨r, hr, hK⟩ (fun _ x => Classical.choose (next x)) n
  have hstep : ∀ n, Ref refs U (p (n + 1)).1 (p n).1 := fun n =>
    ⟨(p n).2.1, (p (n + 1)).2.1, Classical.choose_spec (next (p n))⟩
  have hchain : ∀ i d, TransGen (Ref refs U) (p (i + d + 1)).1 (p i).1 := by
    intro i d
    induction d with
    | zero => exact .single (hstep i)
    | succ d ih => exact TransGen.trans (.single (hstep (i + d + 1))) ih
  obtain ⟨i, j, hij, -, hp⟩ :=
    pigeonhole L (fun n => (p n).1) (fun n _ => hL _ (p n).2.1)
  have := hchain i (j - i - 1)
  rw [show i + (j - i - 1) + 1 = j by omega] at this
  exact hA _ (hp ▸ this)

/-- `K(S) = S` exactly when `S` is closed and acyclic. -/
theorem K_eq_self_iff {S : E → Prop} (hfin : Covered S) :
    (∀ r, K refs S r ↔ S r) ↔ Closed refs S ∧ Acyclic refs S := by
  constructor
  · intro h
    refine ⟨fun r hr x hx => (h x).1 (K_closed S r ((h r).2 hr) x hx), fun a ha => ?_⟩
    exact K_acyclic S a (TransGen.ref_mono (fun r hr => (h r).2 hr) ha)
  · rintro ⟨hC, hA⟩ r
    exact ⟨K_sub, K_greatest hfin (fun _ h => h) hC hA r⟩

theorem Covered.sub {U V : E → Prop} (h : Covered V) (hUV : ∀ r, U r → V r) :
    Covered U :=
  let ⟨L, hL⟩ := h; ⟨L, fun r hr => hL r (hUV r hr)⟩

/-- The kernel is idempotent. -/
theorem K_idem {S : E → Prop} (hfin : Covered S) (r : E) :
    K refs (K refs S) r ↔ K refs S r :=
  ⟨K_sub, K_greatest (hfin.sub fun _ => K_sub) (fun _ h => h) (K_closed S) (K_acyclic S) r⟩

/-! ## `cor:obstructions`: obstructions -/

/-- `r ∈ S` has a reference outside `S` or lies on a reference cycle of `S`. -/
def Bad (S : E → Prop) (r : E) : Prop :=
  S r ∧ ((∃ x ∈ refs r, ¬ S x) ∨ TransGen (Ref refs S) r r)

/-- `r` reaches a bad record (zero-length paths included). -/
def Obstructed (S : E → Prop) (r : E) : Prop := ∃ b, Reach refs S r b ∧ Bad (refs := refs) S b

theorem K_reach {S : E → Prop} {a b : E} (h : Reach refs S a b) (ha : K refs S a) :
    K refs S b := by
  induction h with
  | refl => exact ha
  | head s _ ih => exact ih (K_closed S _ ha _ s.2)

theorem K_transGen {S : E → Prop} {a c : E} (h : TransGen (Ref refs S) a c) (hc : K refs S c) :
    TransGen (Ref refs (K refs S)) a c := by
  induction h with
  | single h => exact .single ⟨hc, K_closed S _ hc _ h.2.2, h.2.2⟩
  | tail _ h ih =>
    have hb := K_closed S _ hc _ h.2.2
    exact .tail (ih hb) ⟨hc, hb, h.2.2⟩

/-- **`cor:obstructions`.** `K(S) = S ∖ A`, where `A` is the set of records reaching an obstruction. -/
theorem K_obstructions {S : E → Prop} (hfin : Covered S) (r : E) :
    K refs S r ↔ S r ∧ ¬ Obstructed (refs := refs) S r := by
  classical
  constructor
  · intro hr
    refine ⟨K_sub hr, fun ⟨b, hrb, hb, hbad⟩ => ?_⟩
    have hKb := K_reach hrb hr
    rcases hbad with ⟨x, hx, hxS⟩ | hcyc
    · exact hxS (K_sub (K_closed S b hKb x hx))
    · exact K_acyclic S b (K_transGen hcyc hKb)
  · rintro ⟨hS, hO⟩
    let U : E → Prop := fun x => S x ∧ ¬ Obstructed (refs := refs) S x
    have hC : Closed refs U := by
      intro x ⟨hxS, hxO⟩ y hy
      have hyS : S y := Classical.byContradiction fun hyS =>
        hxO ⟨x, .refl x, hxS, .inl ⟨y, hy, hyS⟩⟩
      exact ⟨hyS, fun ⟨b, hyb, hb⟩ => hxO ⟨b, .head ⟨hxS, hy⟩ hyb, hb⟩⟩
    have hA : Acyclic refs U := fun a ha =>
      have h := TransGen.ref_mono (V := S) (fun _ h => h.1) ha
      have hU : U a := by cases ha with | single h => exact h.1 | tail _ h => exact h.1
      hU.2 ⟨a, .refl a, hU.1, .inr h⟩
    exact K_greatest (hfin.sub fun _ h => h.1) (fun _ h => h.1) hC hA r ⟨hS, hO⟩

/-! ## Locality -/

/-- If everything a kernel record of `T` reaches inside `T` lies in `S`, the record
is in `K(S)`. -/
theorem K_local {S T : E → Prop} {r : E} (hr : K refs T r)
    (hreach : ∀ x, Reach refs T r x → T x → S x) : K refs S r := by
  obtain ⟨k, hk⟩ := hr
  suffices ∀ k r, iter refs T k r → (∀ x, Reach refs T r x → T x → S x) → iter refs S k r from
    ⟨k, this k r hk hreach⟩
  intro k
  induction k with
  | zero => intro _ h; exact h.elim
  | succ k ih =>
    intro r hr hre
    refine ⟨hre r (.refl r) hr.1, fun y hy => ih y (hr.2 y hy) fun x hx hTx => ?_⟩
    exact hre x (.head ⟨hr.1, hy⟩ hx) hTx

/-! ## `prop:dual`: reference closure -/

/-- `C_D(S)`: the records of `D` reachable from `S`. -/
def Cl (D S : E → Prop) (x : E) : Prop := D x ∧ ∃ s, S s ∧ Reach refs D s x

theorem reach_closed {D : E → Prop} (hD : Closed refs D) {a b : E} (h : Reach refs D a b)
    (ha : D a) : D b := by
  induction h with
  | refl => exact ha
  | head s _ ih => exact ih (hD _ s.1 _ s.2)

/-- **`prop:dual`.** For closed `D` and `S ⊆ D`: `C_D(S)` is defined (every reachable eid
is in `D`), is the least closed superset of `S`, satisfies
`K(S) ⊆ K(C_D(S)) ⊆ C_D(S)`, and a record of `S` gained by `K(C_D(S))` reaches a record
outside `S`. -/
theorem refClosure {D S : E → Prop} (hD : Closed refs D) (hSD : ∀ r, S r → D r) :
    (∀ x s, S s → Reach refs D s x → D x) ∧
    (∀ r, S r → Cl (refs := refs) D S r) ∧
    Closed refs (Cl (refs := refs) D S) ∧
    (∀ U : E → Prop, Closed refs U → (∀ r, S r → U r) → ∀ r, Cl (refs := refs) D S r → U r) ∧
    (∀ r, K refs S r → K refs (Cl (refs := refs) D S) r) ∧
    (∀ r, K refs (Cl (refs := refs) D S) r → Cl (refs := refs) D S r) ∧
    (∀ r, S r → K refs (Cl (refs := refs) D S) r → ¬ K refs S r →
      ∃ x, Reach refs (Cl (refs := refs) D S) r x ∧ Cl (refs := refs) D S x ∧ ¬ S x) := by
  classical
  have hdef : ∀ x s, S s → Reach refs D s x → D x := fun x s hs h => reach_closed hD h (hSD s hs)
  have hsub : ∀ r, S r → Cl (refs := refs) D S r := fun r hr => ⟨hSD r hr, r, hr, .refl r⟩
  refine ⟨hdef, hsub, ?_, ?_, fun r => K_mono hsub, fun r => K_sub, ?_⟩
  · rintro x ⟨hx, s, hs, hsx⟩ y hy
    exact ⟨hD x hx y hy, s, hs, hsx.tail ⟨hx, hy⟩⟩
  · rintro U hU hSU x ⟨-, s, hs, hsx⟩
    suffices ∀ a b, Reach refs D a b → U a → U b from this s x hsx (hSU s hs)
    intro a b h ha
    induction h with
    | refl => exact ha
    | head st _ ih => exact ih (hU _ ha _ st.2)
  · intro r hr hK hnK
    refine Classical.byContradiction fun hno => hnK (K_local hK ?_)
    intro x hx hCx
    exact Classical.byContradiction fun hxS => hno ⟨x, hx, hCx, hxS⟩

/-! ## `prop:bit` (kernel part): the exportable bit never changes -/

/-- Records of `D` reference no record of `D' ∖ D`: then kernel membership of a record of
`D` is the same in `D` and in any extension `D' ⊇ D`. -/
theorem K_extend {D D' : E → Prop} (hDD' : ∀ r, D r → D' r)
    (hold : ∀ r, D r → ∀ x ∈ refs r, D' x → D x) {r : E} (hr : D r) :
    K refs D' r ↔ K refs D r := by
  refine ⟨fun hK => K_local hK fun x hx hx' => ?_, K_mono hDD'⟩
  suffices ∀ a b, Reach refs D' a b → D a → D' b → D b from this r x hx hr hx'
  intro a b h
  induction h with
  | refl => exact fun ha _ => ha
  | head st hbc ih =>
    intro ha hc
    -- the intermediate record is in `D'` because the chain continues from it or ends at it
    rename_i a' b' c'
    have hb' : D' b' := by
      cases hbc with
      | refl => exact hc
      | head st' _ => exact st'.1
    exact ih (hold _ ha _ st.2 hb') hc

/-- For a closed snapshot `Dt ⊆ D`, `K(Dt) = Dt ∩ K(D)`. -/
theorem K_closed_snapshot {D Dt : E → Prop} (hsub : ∀ r, Dt r → D r) (hC : Closed refs Dt)
    (r : E) : K refs Dt r ↔ Dt r ∧ K refs D r := by
  refine ⟨fun h => ⟨K_sub h, K_mono hsub h⟩, fun ⟨hr, hK⟩ => K_local hK fun x hx _ => ?_⟩
  suffices ∀ a b, Reach refs D a b → Dt a → Dt b from this r x hx hr
  intro a b h
  induction h with
  | refl => exact id
  | head st _ ih => exact fun ha => ih (hC _ ha _ st.2)

end StatementExport
