/-
Per-family keys and their order, scan validity and the row predicate of a scan.
Verified module: imports only `Init` and verified modules.

Keys mirror the SQLite index definitions with the rowid (`eid`) appended, so the contract
order is the order SQLite stores. Values compare as signed 64-bit integers (as `Int`), an
absent value sorts before every present one (SQLite NULL ordering), and the descending
`t_add` column of the history indexes is stored negated.
-/
import Tiramemsu.Store.Types

namespace Tiramemsu.Store

-- @lat: [[architecture#Store Abstraction#Index Families]]

/-- Columns of the triple table. -/
inductive Col where
  | s | p | o | tAdd | tRet | vFrom | vTo | retKind | eid
  deriving Repr, DecidableEq, Inhabited

/-- The value of a column; `none` is SQL NULL. -/
def Col.get : Col → TripleRow → Option Int64
  | .s, r => some r.s
  | .p, r => some r.p
  | .o, r => some r.o
  | .tAdd, r => some r.tAdd
  | .tRet, r => r.tRet
  | .vFrom, r => r.vFrom
  | .vTo, r => r.vTo
  | .retKind, r => r.retKind
  | .eid, r => some r.eid

/-- SQL column name. -/
def Col.sqlName : Col → String
  | .s => "s" | .p => "p" | .o => "o" | .tAdd => "t_add" | .tRet => "t_ret"
  | .vFrom => "v_from" | .vTo => "v_to" | .retKind => "ret_kind" | .eid => "eid"

/-- The leading triple permutation of a live or history family. -/
def Family.perm : Family → List Col
  | .liveSpo | .histSpo => [.s, .p, .o]
  | .livePos | .histPos => [.p, .o, .s]
  | .liveOsp | .histOsp => [.o, .s, .p]
  | .validP => [.p]
  | .logAdd | .logRet => []

/-- Key columns with their direction (`true` = descending), `eid` last. -/
def Family.cols : Family → List (Col × Bool)
  | f@(.liveSpo) | f@(.livePos) | f@(.liveOsp) =>
      f.perm.map (·, false) ++ [(.tRet, false), (.vFrom, false), (.vTo, false), (.eid, false)]
  | f@(.histSpo) | f@(.histPos) | f@(.histOsp) =>
      f.perm.map (·, false) ++
        [(.tAdd, true), (.tRet, false), (.vFrom, false), (.vTo, false), (.eid, false)]
  | .validP => [(.p, false), (.vFrom, false), (.vTo, false), (.eid, false)]
  | .logAdd => [(.tAdd, false), (.eid, false)]
  | .logRet => [(.tRet, false), (.retKind, false), (.eid, false)]

/-- The longest equality prefix a family accepts. -/
def Family.maxPrefix : Family → Nat
  | .liveSpo | .livePos | .liveOsp | .histSpo | .histPos | .histOsp => 3
  | .validP => 1
  | .logAdd | .logRet => 0

/-- Families whose index is partial on live rows; they only admit the `now` view. -/
def Family.isLive : Family → Bool
  | .liveSpo | .livePos | .liveOsp | .validP => true
  | _ => false

/-- The partial-index predicate of a family. -/
def Family.rowFilter : Family → TripleRow → Bool
  | .liveSpo, r | .livePos, r | .liveOsp, r | .validP, r => r.tRet.isNone
  | .logRet, r => r.tRet.isSome
  | _, _ => true

/-- A key component: the column value as an integer, negated for descending columns. -/
def keyComp (desc : Bool) (v : Option Int64) : Option Int :=
  v.map fun x => if desc then -x.toInt else x.toInt

/-- The key of a row in a family. -/
def keyOf (f : Family) (r : TripleRow) : List (Option Int) :=
  f.cols.map fun (c, d) => keyComp d (c.get r)

/-- Absent first, then integer order. -/
def cmpOpt : Option Int → Option Int → Ordering
  | none, none => .eq
  | none, some _ => .lt
  | some _, none => .gt
  | some a, some b => compare a b

/-- Lexicographic comparison of keys. -/
def cmpKey : List (Option Int) → List (Option Int) → Ordering
  | [], [] => .eq
  | [], _ :: _ => .lt
  | _ :: _, [] => .gt
  | a :: as, b :: bs =>
    match cmpOpt a b with
    | .eq => cmpKey as bs
    | o => o

/-- Comparison of two rows in a family's key order. -/
def keyCmp (f : Family) (a b : TripleRow) : Ordering := cmpKey (keyOf f a) (keyOf f b)

/-- Strict key order of a family. -/
def keyLt (f : Family) (a b : TripleRow) : Prop := keyCmp f a b = .lt

instance (f : Family) (a b : TripleRow) : Decidable (keyLt f a b) :=
  inferInstanceAs (Decidable (keyCmp f a b = .lt))

/-- Non-strict key order as a boolean (used to sort). -/
def keyLe (f : Family) (a b : TripleRow) : Bool := keyCmp f a b != .gt

/-- Whether a value satisfies a lower bound; an absent value never does. -/
def Bound.admitsLo : Bound → Option Int64 → Bool
  | .incl v, some x => decide (v.toInt ≤ x.toInt)
  | .excl v, some x => decide (v.toInt < x.toInt)
  | _, none => false

/-- Whether a value satisfies an upper bound; an absent value never does. -/
def Bound.admitsHi : Bound → Option Int64 → Bool
  | .incl v, some x => decide (x.toInt ≤ v.toInt)
  | .excl v, some x => decide (x.toInt < v.toInt)
  | _, none => false

/-- Transaction-time predicate (the Rust view mapping). -/
def TxSel.admits : TxSel → TripleRow → Bool
  | .now, r => r.tRet.isNone
  | .asOf t, r =>
    decide (r.tAdd.toInt ≤ t.toInt) &&
      (match r.tRet with
       | none => true
       | some x => decide (t.toInt < x.toInt))
  | .history, _ => true

/-- Valid-time predicate: the half-open interval `[v_from, v_to)`, absent ends unbounded. -/
def ValidSel.admits : ValidSel → TripleRow → Bool
  | .unfiltered, _ => true
  | .at d, r =>
    (match r.vFrom with
     | none => true
     | some x => decide (x.toInt ≤ d.toInt)) &&
    (match r.vTo with
     | none => true
     | some x => decide (d.toInt < x.toInt))

/-- A view admits a row when both parts do. -/
def View.admits (v : View) (r : TripleRow) : Bool := v.tx.admits r && v.valid.admits r

/-- The column a scan's bounds constrain: the first key column after the prefix. -/
def ScanSpec.boundCol (sp : ScanSpec) : Option Col :=
  (sp.family.cols.map Prod.fst)[sp.pre.size]?

/-- Whether the row agrees with the equality prefix. -/
def ScanSpec.prefixOk (sp : ScanSpec) (r : TripleRow) : Bool :=
  ((sp.family.cols.map Prod.fst).zip sp.pre.toList).all fun (c, v) => c.get r == some v

/-- Whether the row satisfies both bounds. -/
def ScanSpec.boundsOk (sp : ScanSpec) (r : TripleRow) : Bool :=
  let val := sp.boundCol.bind (·.get r)
  (match sp.lo with
   | none => true
   | some b => b.admitsLo val) &&
  (match sp.hi with
   | none => true
   | some b => b.admitsHi val)

/-- Whether a scan is valid: prefix within the family's limit, live families only under `now`. -/
def ScanSpec.valid (sp : ScanSpec) : Bool :=
  decide (sp.pre.size ≤ sp.family.maxPrefix) && (!sp.family.isLive || sp.view.tx == .now)

/-- Validity as a proposition. -/
def ScanSpec.Valid (sp : ScanSpec) : Prop := sp.valid = true

/-- The row predicate of a scan: family filter, prefix, bounds and view. -/
def ScanSpec.matches (sp : ScanSpec) (r : TripleRow) : Bool :=
  sp.family.rowFilter r && sp.prefixOk r && sp.boundsOk r && sp.view.admits r

/-- The row predicate as a proposition. -/
def ScanSpec.Matches (sp : ScanSpec) (r : TripleRow) : Prop := sp.matches r = true

end Tiramemsu.Store
