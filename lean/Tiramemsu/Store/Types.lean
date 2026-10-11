/-
Store types: rows of the format-1 tables, views, scan specifications and store errors.
Verified module: imports only `Init`.
-/

namespace Tiramemsu.Store

-- @lat: [[architecture#Store Abstraction]]

/-- A row of the `triple` table. Ids are SQLite INTEGERs; the ObjectId codec arrives in M1. -/
structure TripleRow where
  eid : Int64
  s : Int64
  p : Int64
  o : Int64
  tAdd : Int64
  tRet : Option Int64 := none
  vFrom : Option Int64 := none
  vTo : Option Int64 := none
  retKind : Option Int64 := none
  deriving Repr, DecidableEq, Inhabited

/-- A row of the `term` table. `num` holds the bits of the `REAL` value. -/
structure TermRow where
  id : Int64
  tag : Int64
  lex : String
  dt : Option Int64 := none
  lang : Option String := none
  num : Option UInt64 := none
  deriving Repr, DecidableEq, Inhabited

/-- A row of the `tx` table. -/
structure TxRow where
  t : Int64
  instant : Int64
  deriving Repr, DecidableEq, Inhabited

/-- A row of the `volatile` table, keyed by `(s, key)`. -/
structure VolatileRow where
  s : Int64
  key : Int64
  value : Int64
  updatedAt : Int64
  deriving Repr, DecidableEq, Inhabited

/-- Transaction-time part of a view. -/
inductive TxSel where
  | now
  | asOf (t : Int64)
  | history
  deriving Repr, DecidableEq, Inhabited

/-- Valid-time part of a view. -/
inductive ValidSel where
  | unfiltered
  | at (d : Int64)
  deriving Repr, DecidableEq, Inhabited

/-- A view: a transaction-time selector and a valid-time selector, combined by conjunction. -/
structure View where
  tx : TxSel := .now
  valid : ValidSel := .unfiltered
  deriving Repr, DecidableEq, Inhabited

/-- The index families of the triple table; each mirrors one format-1 index. -/
inductive Family where
  | liveSpo | livePos | liveOsp
  | histSpo | histPos | histOsp
  | validP
  | logAdd | logRet
  deriving Repr, DecidableEq, Inhabited

/-- An inclusive or exclusive bound on the first key column after the prefix. -/
inductive Bound where
  | incl (v : Int64)
  | excl (v : Int64)
  deriving Repr, DecidableEq, Inhabited

/-- A range scan: family, equality prefix, optional bounds on the next key column, view. -/
structure ScanSpec where
  family : Family
  pre : Array Int64 := #[]
  lo : Option Bound := none
  hi : Option Bound := none
  view : View := {}
  deriving Repr, DecidableEq, Inhabited

/-- Contract violations a write can report. -/
inductive Violation where
  | idReused
  | retractOnce
  | termKeyTaken
  | instantTaken
  deriving Repr, DecidableEq, Inhabited

/-- Every store failure. The model store never produces `sqlite`. -/
inductive StoreError where
  | violation (v : Violation)
  | notFound
  | invalidScan
  | misuse (what : String)
  | sqlite (code ext : UInt32) (msg : String)
  deriving Repr, DecidableEq, Inhabited

instance : ToString StoreError where
  toString
    | .violation v => s!"violation {repr v}"
    | .notFound => "not found"
    | .invalidScan => "invalid scan"
    | .misuse w => s!"misuse: {w}"
    | .sqlite c e m => s!"sqlite error {c}/{e}: {m}"

end Tiramemsu.Store
