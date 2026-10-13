import StatementExport.Kernel

/-!
# Temporal operations and the full history

A history is a list of insert batches, oldest first. An ordinary insert or a lineage
record is a batch that references only earlier batches (C3, C6 lineage); a replay batch
may also reference its own records (C6). Batches allocate fresh eids and records are
never removed from the history (C4); retraction only sets `d` and does not change
references, so it does not appear here. This file proves `prop:history` (closed
history, cycles inside one replay), `prop:strict` (acyclic-replay rule) and, for live
sets, `prop:closure` (snapshot closure under inserts and cascading retraction).
-/

namespace StatementExport

open Relation

variable {E : Type}

structure Batch (E : Type) where
  recs : List E
  replay : Bool

/-- The records inserted by a history. -/
def Ins (hs : List (Batch E)) (x : E) : Prop := ∃ b ∈ hs, x ∈ b.recs

section
variable (refs : E → List E)

/-- Histories that C3–C6 can produce. -/
inductive Valid : List (Batch E) → Prop
  | nil : Valid []
  | snoc {hs : List (Batch E)} {b : Batch E} : Valid hs →
      (∀ x ∈ b.recs, ¬ Ins hs x) →
      (∀ r ∈ b.recs, ∀ x ∈ refs r, Ins hs x ∨ (b.replay = true ∧ x ∈ b.recs)) →
      Valid (hs ++ [b])

/-- Valid histories whose replay batches passed the acyclic-replay check. -/
def ReplayChecked (hs : List (Batch E)) : Prop :=
  ∀ b ∈ hs, b.replay = true → Acyclic refs (· ∈ b.recs)

end

variable {refs : E → List E}

theorem ins_snoc {hs : List (Batch E)} {b : Batch E} {x : E} :
    Ins (hs ++ [b]) x ↔ Ins hs x ∨ x ∈ b.recs := by
  constructor
  · rintro ⟨c, hc, hx⟩
    rcases List.mem_append.1 hc with hc | hc
    · exact .inl ⟨c, hc, hx⟩
    · rw [List.mem_singleton.1 hc] at hx; exact .inr hx
  · rintro (⟨c, hc, hx⟩ | hx)
    · exact ⟨c, List.mem_append_left _ hc, hx⟩
    · exact ⟨b, List.mem_append_right _ (List.mem_singleton_self _), hx⟩

theorem ins_covered (hs : List (Batch E)) : Covered (Ins hs) :=
  ⟨hs.flatMap Batch.recs, fun _ ⟨b, hb, hx⟩ => List.mem_flatMap.2 ⟨b, hb, hx⟩⟩

/-- **`prop:history` (closure).** The history is closed after every batch. -/
theorem history_closed {hs : List (Batch E)} (h : Valid refs hs) : Closed refs (Ins hs) := by
  induction h with
  | nil => intro _ ⟨_, hb, _⟩; nomatch hb
  | snoc _ _ hrefs ih =>
    intro r hr x hx
    rcases ins_snoc.1 hr with hr | hr
    · exact ins_snoc.2 (.inl (ih r hr x hx))
    · rcases hrefs r hr x hx with h | ⟨_, h⟩
      · exact ins_snoc.2 (.inl h)
      · exact ins_snoc.2 (.inr h)

/-- **`prop:history` (cycles).** Every reference cycle of the history lies inside one
replay batch. -/
theorem history_cycle {hs : List (Batch E)} (h : Valid refs hs) {a : E}
    (hc : TransGen (Ref refs (Ins hs)) a a) :
    ∃ b ∈ hs, b.replay = true ∧ a ∈ b.recs ∧ TransGen (Ref refs (· ∈ b.recs)) a a := by
  induction h generalizing a with
  | nil => cases hc with | single h => exact absurd h.1 (by rintro ⟨_, h, _⟩; nomatch h)
                         | tail _ h => exact absurd h.1 (by rintro ⟨_, h, _⟩; nomatch h)
  | @snoc hs b hv hfresh hrefs ih =>
    have hC := history_closed hv
    -- a record of the old history references only old records
    have down : ∀ x y, TransGen (Ref refs (Ins (hs ++ [b]))) x y → Ins hs y →
        TransGen (Ref refs (Ins hs)) x y := by
      intro x y hxy
      induction hxy with
      | single h => exact fun hy => .single ⟨hy, hC _ hy _ h.2.2, h.2.2⟩
      | tail _ h ih' =>
        intro hy
        have hz := hC _ hy _ h.2.2
        exact .tail (ih' hz) ⟨hy, hz, h.2.2⟩
    -- a referrer of a new record is new
    have up : ∀ u v, Ref refs (Ins (hs ++ [b])) u v → u ∈ b.recs → v ∈ b.recs := by
      intro u v huv hu
      rcases ins_snoc.1 huv.1 with hv' | hv'
      · exact absurd (hC _ hv' _ huv.2.2) (hfresh u hu)
      · exact hv'
    have within : ∀ x y, TransGen (Ref refs (Ins (hs ++ [b]))) x y → x ∈ b.recs →
        y ∈ b.recs ∧ TransGen (Ref refs (· ∈ b.recs)) x y := by
      intro x y hxy
      induction hxy with
      | single h =>
        exact fun hx => have hy := up _ _ h hx; ⟨hy, .single ⟨hy, hx, h.2.2⟩⟩
      | tail _ h ih' =>
        intro hx
        obtain ⟨hz, hxz⟩ := ih' hx
        have hy := up _ _ h hz
        exact ⟨hy, .tail hxz ⟨hy, hz, h.2.2⟩⟩
    have ha : Ins (hs ++ [b]) a := by
      cases hc with | single h => exact h.1 | tail _ h => exact h.1
    rcases ins_snoc.1 ha with ha | ha
    · obtain ⟨c, hc', hrep, hac, hcyc⟩ := ih (down a a hc ha)
      exact ⟨c, List.mem_append_left _ hc', hrep, hac, hcyc⟩
    · obtain ⟨-, hcyc⟩ := within a a hc ha
      have hrep : b.replay = true := by
        cases hcyc with
        | single h | tail _ h =>
          rcases hrefs a ha _ h.2.2 with h' | ⟨h', -⟩
          · exact absurd h' (hfresh _ h.2.1)
          · exact h'
      exact ⟨b, List.mem_append_right _ (List.mem_singleton_self _), hrep, ha, hcyc⟩

/-- **`prop:history` (kernel).** `K(D)` is `D` minus the records that reach a cycle inside a
replay batch. -/
theorem history_kernel {hs : List (Batch E)} (h : Valid refs hs) (r : E) :
    K refs (Ins hs) r ↔ Ins hs r ∧ ¬ ∃ c, Reach refs (Ins hs) r c ∧
      ∃ b ∈ hs, b.replay = true ∧ c ∈ b.recs ∧ TransGen (Ref refs (· ∈ b.recs)) c c := by
  rw [K_obstructions (ins_covered hs)]
  refine and_congr_right fun _ => not_congr ⟨?_, ?_⟩
  · rintro ⟨c, hrc, hc, hbad⟩
    rcases hbad with ⟨x, hx, hxS⟩ | hcyc
    · exact absurd (history_closed h c hc x hx) hxS
    · exact ⟨c, hrc, history_cycle h hcyc⟩
  · rintro ⟨c, hrc, b, hb, -, hcb, hcyc⟩
    exact ⟨c, hrc, ⟨b, hb, hcb⟩, .inr (TransGen.ref_mono (fun x hx => ⟨b, hb, hx⟩) hcyc)⟩

/-- **`prop:strict`.** Under the acyclic-replay rule the history is closed and acyclic,
so `K(D) = D` and the full history exports with no omissions. -/
theorem history_acyclic {hs : List (Batch E)} (h : Valid refs hs)
    (hrule : ReplayChecked refs hs) :
    Closed refs (Ins hs) ∧ Acyclic refs (Ins hs) ∧ ∀ r, K refs (Ins hs) r ↔ Ins hs r := by
  have hA : Acyclic refs (Ins hs) := fun a hc =>
    let ⟨b, hb, hrep, _, hcyc⟩ := history_cycle h hc
    hrule b hb hrep a hcyc
  exact ⟨history_closed h, hA, (K_eq_self_iff (ins_covered hs)).2 ⟨history_closed h, hA⟩⟩

/-- Without replay batches (the restricted protocol) the history is acyclic. -/
theorem restricted_acyclic {hs : List (Batch E)} (h : Valid refs hs)
    (hnr : ∀ b ∈ hs, b.replay = false) : Acyclic refs (Ins hs) := fun a hc =>
  let ⟨b, hb, hrep, _⟩ := history_cycle h hc
  by rw [hnr b hb] at hrep; exact Bool.false_ne_true hrep

/-! ## `prop:closure`: committed snapshots under the restricted protocol -/

/-- An insert that references only live records (C3) keeps the live set closed. -/
theorem insert_closed {Live : E → Prop} {r : E} (hC : Closed refs Live)
    (hr : ∀ x ∈ refs r, Live x) : Closed refs (fun y => Live y ∨ y = r) := by
  rintro y (hy | rfl) x hx
  · exact .inl (hC y hy x hx)
  · exact .inl (hr x hx)

/-- A retraction set that contains every live dependent of its members (C5) keeps the
live set closed. -/
theorem retract_closed {Live R : E → Prop} (hC : Closed refs Live)
    (hcascade : ∀ y, Live y → ¬ R y → ∀ x ∈ refs y, ¬ R x) :
    Closed refs (fun y => Live y ∧ ¬ R y) :=
  fun y ⟨hy, hny⟩ x hx => ⟨hC y hy x hx, hcascade y hy hny x hx⟩

/-- Operations of the restricted protocol on the live set. -/
inductive Op (E : Type) where
  | insert (r : E)
  | retract (R : E → Prop)

section
variable (refs : E → List E)

/-- Apply an operation if its precondition (C3 or C5) holds. -/
inductive Apply : (E → Prop) → Op E → (E → Prop) → Prop
  | insert {Live : E → Prop} {r : E} : (∀ x ∈ refs r, Live x) →
      Apply Live (.insert r) (fun y => Live y ∨ y = r)
  | retract {Live R : E → Prop} : (∀ y, Live y → ¬ R y → ∀ x ∈ refs y, ¬ R x) →
      Apply Live (.retract R) (fun y => Live y ∧ ¬ R y)

/-- Run a list of operations from a live set. -/
inductive Run : (E → Prop) → List (Op E) → (E → Prop) → Prop
  | nil {Live : E → Prop} : Run Live [] Live
  | cons {Live Mid Out : E → Prop} {o : Op E} {os : List (Op E)} :
      Apply refs Live o Mid → Run Mid os Out → Run Live (o :: os) Out

end

/-- **`prop:closure` (snapshots).** Starting from the empty store, every live set reached
by inserts obeying C3 and retractions obeying C5 is closed, hence so is every committed
snapshot. -/
theorem snapshot_closed {os : List (Op E)} {Live : E → Prop}
    (h : Run refs (fun _ => False) os Live) : Closed refs Live := by
  suffices ∀ L₀ os L, Run refs L₀ os L → Closed refs L₀ → Closed refs L from
    this _ _ _ h (fun _ h => h.elim)
  intro L₀ os L h
  induction h with
  | nil => exact id
  | cons ha _ ih =>
    intro hC
    apply ih
    cases ha with
    | insert hr => exact insert_closed hC hr
    | retract hc => exact retract_closed hC hc

end StatementExport
