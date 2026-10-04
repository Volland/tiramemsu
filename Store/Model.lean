/-
`ModelStore`: the pure store every theorem is stated over.
Verified module: imports only `Init` and verified modules.

A state is a set of lists of rows. Scans filter by the scan predicate and sort by the family
key; writes check their violation first and, on error, leave the state as it was (the error
is returned in `Except`, so `tryCatch` in `StateT` restores the previous state, mirroring
SQLite's statement-level abort).
-/
import Tiramemsu.Store.Order
import Tiramemsu.Store.Interface

namespace Tiramemsu.Store

-- @lat: [[architecture#Store Abstraction#Model Store]]

/-- The contents of a store: one list per table. -/
structure ModelState where
  triples : List TripleRow := []
  terms : List TermRow := []
  txs : List TxRow := []
  counters : List (String × Int64) := []
  volatile : List VolatileRow := []
  predMulti : List Int64 := []
  deriving Repr, DecidableEq, Inhabited

/-- Well-formed: statement ids are unique. -/
def ModelState.WF (st : ModelState) : Prop :=
  st.triples.Pairwise fun a b => a.eid ≠ b.eid

/-- The writer's view of the model: committed state, current state, transaction flag and the
savepoint stack (newest first, each with the current state when it was opened). -/
structure ModelStore where
  committed : ModelState := {}
  current : ModelState := {}
  inTx : Bool := false
  sps : List (String × ModelState) := []
  deriving Repr, Inhabited

/-- A fresh store over a committed state. -/
def ModelStore.ofState (st : ModelState) : ModelStore := { committed := st, current := st }

/-! ## Reads -/

/-- Ordered fold with early exit; the meaning of every scan. -/
def foldWithExit {m : Type → Type} [Monad m] {α β : Type} :
    List α → β → (β → α → m (ForInStep β)) → m β
  | [], b, _ => pure b
  | x :: xs, b, f => do
    match ← f b x with
    | .done b' => pure b'
    | .yield b' => foldWithExit xs b' f

/-- The rows of a scan, in key order. -/
def ModelState.scanList (st : ModelState) (sp : ScanSpec) : List TripleRow :=
  (st.triples.filter sp.matches).mergeSort (keyLe sp.family)

/-- A scan that checks validity first. -/
def ModelState.scanList? (st : ModelState) (sp : ScanSpec) : Except StoreError (List TripleRow) :=
  if sp.valid then .ok (st.scanList sp) else .error .invalidScan

def ModelState.triple (st : ModelState) (eid : Int64) : Option TripleRow :=
  st.triples.find? (·.eid == eid)

def ModelState.termById (st : ModelState) (id : Int64) : Option TermRow :=
  st.terms.find? (·.id == id)

def ModelState.termByKey (st : ModelState) (tag : Int64) (lex : String) (dt : Option Int64)
    (lang : Option String) : Option Int64 :=
  (st.terms.find? fun t => t.tag == tag && t.lex == lex && t.dt == dt && t.lang == lang).map (·.id)

def ModelState.txByT (st : ModelState) (t : Int64) : Option TxRow :=
  st.txs.find? (·.t == t)

/-- The transaction with the greatest instant at or before `i`. -/
def ModelState.txAtOrBefore (st : ModelState) (i : Int64) : Option TxRow :=
  st.txs.foldl (init := none) fun best x =>
    if x.instant.toInt ≤ i.toInt then
      match best with
      | none => some x
      | some b => if b.instant.toInt < x.instant.toInt then some x else best
    else best

def ModelState.counter (st : ModelState) (name : String) : Option Int64 :=
  (st.counters.find? (·.1 == name)).map (·.2)

def ModelState.volatileGet (st : ModelState) (s key : Int64) : Option VolatileRow :=
  st.volatile.find? fun v => v.s == s && v.key == key

def ModelState.volatileOf (st : ModelState) (s : Int64) : Array VolatileRow :=
  ((st.volatile.filter (·.s == s)).mergeSort fun a b => decide (a.key.toInt ≤ b.key.toInt)).toArray

def ModelState.isPredMulti (st : ModelState) (p : Int64) : Bool :=
  st.predMulti.contains p

/-- Scan in any monad with store errors: validity check, then the ordered fold. -/
def ModelState.scanM {m : Type → Type} [Monad m] [MonadExcept StoreError m] {β : Type}
    (st : ModelState) (sp : ScanSpec) (init : β) (f : β → TripleRow → m (ForInStep β)) : m β :=
  match st.scanList? sp with
  | .error e => throw e
  | .ok xs => foldWithExit xs init f

/-! ## Writes on a state -/

/-- SQLite's REAL normalization: NaN is stored as NULL, −0.0 is read back as +0.0. -/
def normalizeNum : Option UInt64 → Option UInt64
  | none => none
  | some b =>
    if (b >>> 52) &&& 0x7FF == 0x7FF && b &&& 0xFFFFFFFFFFFFF != 0 then none
    else if b == 0x8000000000000000 then some 0
    else some b

/-- Term-key equality as the `term_key` unique index sees it (`ifnull(dt, 0)`, `ifnull(lang, '')`). -/
def termKeyEq (a b : TermRow) : Bool :=
  a.tag == b.tag && a.lex == b.lex && a.dt.getD 0 == b.dt.getD 0 && a.lang.getD "" == b.lang.getD ""

/-- The retraction of one row. -/
def retractRow (eid t kind : Int64) (x : TripleRow) : TripleRow :=
  if x.eid == eid then { x with tRet := some t, retKind := some kind } else x

def ModelState.insertTriple (r : TripleRow) (st : ModelState) : Except StoreError ModelState :=
  if st.triples.any (·.eid == r.eid) then .error (.violation .idReused)
  else .ok { st with triples := st.triples ++ [r] }

def ModelState.retract (eid t kind : Int64) (st : ModelState) : Except StoreError ModelState :=
  match st.triple eid with
  | none => .error .notFound
  | some r =>
    if r.tRet.isSome then .error (.violation .retractOnce)
    else .ok { st with triples := st.triples.map (retractRow eid t kind) }

def ModelState.insertTerm (r : TermRow) (st : ModelState) : Except StoreError ModelState :=
  let r := { r with num := normalizeNum r.num }
  if st.terms.any (·.id == r.id) then .error (.violation .idReused)
  else if st.terms.any (termKeyEq r) then .error (.violation .termKeyTaken)
  else .ok { st with terms := st.terms ++ [r] }

def ModelState.insertTx (r : TxRow) (st : ModelState) : Except StoreError ModelState :=
  if st.txs.any (·.t == r.t) then .error (.violation .idReused)
  else if st.txs.any (·.instant == r.instant) then .error (.violation .instantTaken)
  else .ok { st with txs := st.txs ++ [r] }

def ModelState.setCounter (name : String) (v : Int64) (st : ModelState) :
    Except StoreError ModelState :=
  .ok { st with
    counters :=
      if st.counters.any (·.1 == name) then st.counters.map fun kv => if kv.1 == name then (kv.1, v) else kv
      else st.counters ++ [(name, v)] }

def ModelState.volatilePut (r : VolatileRow) (st : ModelState) : Except StoreError ModelState :=
  .ok { st with volatile := st.volatile.filter (fun v => !(v.s == r.s && v.key == r.key)) ++ [r] }

def ModelState.volatileDel (s key : Int64) (st : ModelState) : Except StoreError ModelState :=
  .ok { st with volatile := st.volatile.filter fun v => !(v.s == s && v.key == key) }

def ModelState.addPredMulti (p : Int64) (st : ModelState) : Except StoreError ModelState :=
  .ok { st with predMulti := if st.predMulti.contains p then st.predMulti else st.predMulti ++ [p] }

/-! ## Operations on the store -/

/-- Every state-changing operation of the interface. -/
inductive WriteOp where
  | insertTriple (r : TripleRow)
  | retract (eid t kind : Int64)
  | insertTerm (r : TermRow)
  | insertTx (r : TxRow)
  | setCounter (name : String) (v : Int64)
  | volatilePut (r : VolatileRow)
  | volatileDel (s key : Int64)
  | addPredMulti (p : Int64)
  | begin
  | commit
  | rollback
  | savepoint (name : String)
  | rollbackTo (name : String)
  | release (name : String)
  deriving Repr, DecidableEq, Inhabited

/-- Data writes: the operations that change table contents. -/
def WriteOp.isData : WriteOp → Bool
  | .insertTriple _ | .retract .. | .insertTerm _ | .insertTx _ | .setCounter .. | .volatilePut _
  | .volatileDel .. | .addPredMulti _ => true
  | _ => false

/-- The data step of a data write. -/
def WriteOp.step : WriteOp → ModelState → Except StoreError ModelState
  | .insertTriple r => ModelState.insertTriple r
  | .retract e t k => ModelState.retract e t k
  | .insertTerm r => ModelState.insertTerm r
  | .insertTx r => ModelState.insertTx r
  | .setCounter n v => ModelState.setCounter n v
  | .volatilePut r => ModelState.volatilePut r
  | .volatileDel s k => ModelState.volatileDel s k
  | .addPredMulti p => ModelState.addPredMulti p
  | _ => .ok

/-- The newest savepoint of a name: its state, and the stack with it on top. -/
def spRollback (n : String) : List (String × ModelState) → Option (ModelState × List (String × ModelState))
  | [] => none
  | (m, c) :: rest => if m == n then some (c, (m, c) :: rest) else spRollback n rest

/-- The stack below the newest savepoint of a name. -/
def spRelease (n : String) : List (String × ModelState) → Option (List (String × ModelState))
  | [] => none
  | (m, _) :: rest => if m == n then some rest else spRelease n rest

/-- One operation on the store. Misuse and violations return an error. -/
def WriteOp.run (op : WriteOp) (s : ModelStore) : Except StoreError ModelStore :=
  match op with
  | .begin =>
    if s.inTx then .error (.misuse "begin inside a transaction")
    else .ok { s with inTx := true, current := s.committed, sps := [] }
  | .commit =>
    if s.inTx then .ok { committed := s.current, current := s.current, inTx := false, sps := [] }
    else .error (.misuse "commit without a transaction")
  | .rollback =>
    if s.inTx then .ok { committed := s.committed, current := s.committed, inTx := false, sps := [] }
    else .error (.misuse "rollback without a transaction")
  | .savepoint n =>
    if s.inTx then .ok { s with sps := (n, s.current) :: s.sps }
    else .error (.misuse "savepoint outside a transaction")
  | .rollbackTo n =>
    if s.inTx then
      match spRollback n s.sps with
      | some (c, sps) => .ok { s with current := c, sps := sps }
      | none => .error (.misuse s!"no open savepoint {n}")
    else .error (.misuse "rollback to savepoint outside a transaction")
  | .release n =>
    if s.inTx then
      match spRelease n s.sps with
      | some sps => .ok { s with sps := sps }
      | none => .error (.misuse s!"no open savepoint {n}")
    else .error (.misuse "release outside a transaction")
  | op =>
    if s.inTx then
      match op.step s.current with
      | .ok c => .ok { s with current := c }
      | .error e => .error e
    else .error (.misuse "write outside a transaction")

/-- Runs operations in order, stopping at the first error. -/
def WriteOp.runAll : List WriteOp → ModelStore → Except StoreError ModelStore
  | [], s => .ok s
  | op :: ops, s =>
    match op.run s with
    | .ok s' => WriteOp.runAll ops s'
    | .error e => .error e

/-! ## Interface instances -/

/-- The writer monad of the model. -/
abbrev ModelM := StateT ModelStore (Except StoreError)

/-- The reader monad of a model snapshot. -/
abbrev SnapM := ReaderT ModelState (Except StoreError)

/-- An operation as a monadic action. -/
def WriteOp.exec (op : WriteOp) : ModelM Unit :=
  fun s => match op.run s with
    | .ok s' => .ok ((), s')
    | .error e => .error e

/-- A read of the current state. -/
def ModelM.read {α : Type} (f : ModelState → α) : ModelM α :=
  fun s => .ok (f s.current, s)

instance : ReadStore ModelM where
  scan sp init f := fun s => (s.current.scanM sp init f) s
  triple e := ModelM.read (·.triple e)
  termById i := ModelM.read (·.termById i)
  termByKey tag lex dt lang := ModelM.read (·.termByKey tag lex dt lang)
  txByT t := ModelM.read (·.txByT t)
  txAtOrBefore i := ModelM.read (·.txAtOrBefore i)
  counter n := ModelM.read (·.counter n)
  volatileGet s k := ModelM.read (·.volatileGet s k)
  volatileOf s := ModelM.read (·.volatileOf s)
  predMulti p := ModelM.read (·.isPredMulti p)

instance : WriteStore ModelM where
  insertTriple r := (WriteOp.insertTriple r).exec
  retract e t k := (WriteOp.retract e t k).exec
  insertTerm r := (WriteOp.insertTerm r).exec
  insertTx r := (WriteOp.insertTx r).exec
  setCounter n v := (WriteOp.setCounter n v).exec
  volatilePut r := (WriteOp.volatilePut r).exec
  volatileDel s k := (WriteOp.volatileDel s k).exec
  addPredMulti p := (WriteOp.addPredMulti p).exec
  begin := WriteOp.begin.exec
  commit := WriteOp.commit.exec
  rollback := WriteOp.rollback.exec
  savepoint n := (WriteOp.savepoint n).exec
  rollbackTo n := (WriteOp.rollbackTo n).exec
  release n := (WriteOp.release n).exec

instance : ReadStore SnapM where
  scan sp init f := fun st => (st.scanM sp init f) st
  triple e := fun st => .ok (st.triple e)
  termById i := fun st => .ok (st.termById i)
  termByKey tag lex dt lang := fun st => .ok (st.termByKey tag lex dt lang)
  txByT t := fun st => .ok (st.txByT t)
  txAtOrBefore i := fun st => .ok (st.txAtOrBefore i)
  counter n := fun st => .ok (st.counter n)
  volatileGet s k := fun st => .ok (st.volatileGet s k)
  volatileOf s := fun st => .ok (st.volatileOf s)
  predMulti p := fun st => .ok (st.isPredMulti p)

/-- A model snapshot is the committed state when reading began. -/
instance : SnapshotStore ModelM ModelState SnapM where
  beginRead := fun s => .ok (s.committed, s)
  endRead _ := pure ()
  withSnapshot st act := fun s => match act st with
    | .ok a => .ok (a, s)
    | .error e => .error e

end Tiramemsu.Store
