import StatementExport.Kernel

/-!
# Identity-preserving direct export

Records carry content `(s, p, o)`: subject and object are terms or eids, the predicate is
a term. A Statement Graph is modelled with vertices identified by their labels (an eid for
an internal vertex, a target term for a leaf), so label injectivity holds by construction,
and with one target per role, so each internal vertex has exactly one outgoing edge per
role. Leaves are the role targets, so each has an incoming edge. This file proves
`thm:criterion`: a direct export exists exactly for closed, acyclic sets, it is unique, and its
role targets determine the content.
-/

namespace StatementExport

open Relation

inductive Node (T E : Type) where
  | term (t : T)
  | eid (e : E)
  deriving DecidableEq

structure Content (T E : Type) where
  s : Node T E
  p : T
  o : Node T E

inductive Role where
  | s | p | o

inductive Vertex (E L : Type) where
  | internal (e : E)
  | leaf (l : L)
  deriving DecidableEq

variable {T E L : Type}

def Node.refs : Node T E → List E
  | .term _ => []
  | .eid x => [x]

/-- The references of a record: the eid endpoints of its content. -/
def crefs (c : E → Content T E) (r : E) : List E := (c r).s.refs ++ (c r).o.refs

/-- The role fields of a record, as nodes. -/
def field (c : E → Content T E) (r : E) : Role → Node T E
  | .s => (c r).s
  | .p => .term (c r).p
  | .o => (c r).o

/-- Target of a role edge: the internal vertex of an eid, or the leaf `f(t)`. -/
def encNode (f : T → L) : Node T E → Vertex E L
  | .term t => .leaf (f t)
  | .eid x => .internal x

structure SGraph (E L : Type) where
  internal : E → Prop
  edge : E → Role → Vertex E L

/-- `G.Edge y x`: the internal vertex `x` has a role edge to the internal vertex `y`. -/
def SGraph.Edge (G : SGraph E L) (y x : E) : Prop :=
  G.internal x ∧ ∃ ρ, G.edge x ρ = .internal y

/-- Statement Graph conditions not already guaranteed by the representation: role edges
target existing vertices, and the graph is acyclic. -/
def SGraph.IsSG (G : SGraph E L) : Prop :=
  (∀ x, G.internal x → ∀ ρ y, G.edge x ρ = .internal y → G.internal y) ∧
  ∀ a, ¬ TransGen G.Edge a a

/-- Definition (identity-preserving direct export): (a) internal vertices are exactly the
records of `S`; (b) every role edge targets the referenced occurrence or the leaf `f(t)`. -/
def DirectExport (c : E → Content T E) (f : T → L) (S : E → Prop) (G : SGraph E L) : Prop :=
  (∀ e, G.internal e ↔ S e) ∧ ∀ r, S r → ∀ ρ, G.edge r ρ = encNode f (field c r ρ)

/-- The canonical export `SG(S)`. -/
def SG (c : E → Content T E) (f : T → L) (S : E → Prop) : SGraph E L :=
  ⟨S, fun r ρ => encNode f (field c r ρ)⟩

theorem mem_crefs {c : E → Content T E} {r y : E} :
    y ∈ crefs c r ↔ ∃ ρ, field c r ρ = .eid y := by
  unfold crefs
  constructor
  · intro h
    rcases List.mem_append.1 h with h | h
    · cases hs : (c r).s with
      | term _ => rw [hs] at h; nomatch h
      | eid x => rw [hs] at h; cases List.mem_singleton.1 h; exact ⟨.s, hs⟩
    · cases ho : (c r).o with
      | term _ => rw [ho] at h; nomatch h
      | eid x => rw [ho] at h; cases List.mem_singleton.1 h; exact ⟨.o, ho⟩
  · rintro ⟨ρ, h⟩
    cases ρ with
    | s => simp only [field] at h; simp [h, Node.refs]
    | p => simp [field] at h
    | o => simp only [field] at h; simp [h, Node.refs]

theorem encNode_internal {f : T → L} {n : Node T E} {y : E} :
    encNode f n = .internal y ↔ n = .eid y := by
  cases n <;> simp [encNode]

theorem TransGen.lift {α : Type} {r s : α → α → Prop} (h : ∀ a b, r a b → s a b) {a b : α}
    (hab : TransGen r a b) : TransGen s a b := by
  induction hab with
  | single h' => exact .single (h _ _ h')
  | tail _ h' ih => exact .tail ih (h _ _ h')

/-- **`thm:criterion` (criterion).** `S` has an identity-preserving direct export iff `S` is
closed and acyclic. -/
theorem direct_export_iff (c : E → Content T E) (f : T → L) (S : E → Prop) :
    (∃ G : SGraph E L, G.IsSG ∧ DirectExport c f S G) ↔
      Closed (crefs c) S ∧ Acyclic (crefs c) S := by
  constructor
  · rintro ⟨G, ⟨htgt, hacyc⟩, hint, hedge⟩
    have hC : Closed (crefs c) S := by
      intro r hr x hx
      obtain ⟨ρ, hρ⟩ := mem_crefs.1 hx
      have := htgt r ((hint r).2 hr) ρ x (by rw [hedge r hr ρ, hρ]; rfl)
      exact (hint x).1 this
    refine ⟨hC, fun a ha => hacyc a (TransGen.lift ?_ ha)⟩
    rintro y x ⟨hx, -, hy⟩
    obtain ⟨ρ, hρ⟩ := mem_crefs.1 hy
    exact ⟨(hint x).2 hx, ρ, by rw [hedge x hx ρ, hρ]; rfl⟩
  · rintro ⟨hC, hA⟩
    refine ⟨SG c f S, ⟨?_, fun a ha => hA a (TransGen.lift ?_ ha)⟩,
      fun _ => Iff.rfl, fun _ _ _ => rfl⟩
    · intro x hx ρ y hy
      exact hC x hx y (mem_crefs.2 ⟨ρ, encNode_internal.1 hy⟩)
    · rintro y x ⟨hx, ρ, hρ⟩
      have hy : y ∈ crefs c x := mem_crefs.2 ⟨ρ, encNode_internal.1 hρ⟩
      exact ⟨hx, hC x hx y hy, hy⟩

/-- **`thm:criterion` (uniqueness).** Conditions (a) and (b) fix every vertex and edge. -/
theorem direct_export_unique {c : E → Content T E} {f : T → L} {S : E → Prop}
    {G G' : SGraph E L} (h : DirectExport c f S G) (h' : DirectExport c f S G') :
    (∀ e, G.internal e ↔ G'.internal e) ∧ ∀ r, S r → ∀ ρ, G.edge r ρ = G'.edge r ρ :=
  ⟨fun e => (h.1 e).trans (h'.1 e).symm, fun r hr ρ => (h.2 r hr ρ).trans (h'.2 r hr ρ).symm⟩

theorem encNode_inj {f : T → L} (hf : ∀ a b, f a = f b → a = b) {n n' : Node T E}
    (h : encNode f n = encNode f n') : n = n' := by
  cases n <;> cases n' <;> simp_all [encNode]
  exact hf _ _ h

/-- **`thm:criterion` (recovery).** For injective `f`, the role targets of a direct export
determine the content of every record: reading them recovers `erase(S)`. -/
theorem direct_export_recovers {c c' : E → Content T E} {f : T → L}
    (hf : ∀ a b, f a = f b → a = b) {S : E → Prop} {G : SGraph E L}
    (h : DirectExport c f S G) (h' : DirectExport c' f S G) {r : E} (hr : S r) : c r = c' r := by
  have key : ∀ ρ, field c r ρ = field c' r ρ := fun ρ =>
    encNode_inj hf ((h.2 r hr ρ).symm.trans (h'.2 r hr ρ))
  have hs := key .s; have hp := key .p; have ho := key .o
  simp only [field, Node.term.injEq] at hs hp ho
  cases hc : c r; cases hc' : c' r
  simp_all

end StatementExport
