/-
The abstract store interface. The engine reaches storage only through these classes, which
have exactly two implementations: `ModelStore` (pure, every theorem is stated over it) and
`SqliteStore` (leansqlite, linked to the model by refinement tests).
Verified module: imports only `Init` and verified modules.
-/
import Tiramemsu.Store.Types

namespace Tiramemsu.Store

-- @lat: [[architecture#Store Abstraction#Interface]]

/-- Reads: ordered range scans with early exit, and point reads. A missing row is `none`. -/
class ReadStore (m : Type → Type) where
  /-- Ordered fold over the rows a scan matches; `.done` stops the scan. -/
  scan {β : Type} : ScanSpec → β → (β → TripleRow → m (ForInStep β)) → m β
  /-- A triple row by statement id. -/
  triple : Int64 → m (Option TripleRow)
  /-- A term row by id. -/
  termById : Int64 → m (Option TermRow)
  /-- A term id by key; an absent datatype or language matches only an absent one. -/
  termByKey : (tag : Int64) → (lex : String) → Option Int64 → Option String → m (Option Int64)
  /-- A transaction row by number. -/
  txByT : Int64 → m (Option TxRow)
  /-- The transaction with the greatest instant at or before the given instant. -/
  txAtOrBefore : (instant : Int64) → m (Option TxRow)
  /-- A named counter of `meta`. -/
  counter : String → m (Option Int64)
  /-- A volatile row by `(s, key)`. -/
  volatileGet : (s key : Int64) → m (Option VolatileRow)
  /-- All volatile rows of a subject, ordered by key. -/
  volatileOf : (s : Int64) → m (Array VolatileRow)
  /-- Whether a predicate is in the multi-eid set. -/
  predMulti : Int64 → m Bool

/-- Writes, write transactions and savepoints. Every write needs an open transaction. -/
class WriteStore (m : Type → Type) extends ReadStore m where
  /-- Appends a triple row; a reused statement id is `idReused`. -/
  insertTriple : TripleRow → m Unit
  /-- The single retraction update: sets `t_ret` and `ret_kind` of a live row. -/
  retract : (eid t kind : Int64) → m Unit
  /-- Appends a term row (numeric value normalized as SQLite stores a REAL). -/
  insertTerm : TermRow → m Unit
  /-- Appends a transaction row. -/
  insertTx : TxRow → m Unit
  /-- Sets a named counter. -/
  setCounter : String → Int64 → m Unit
  /-- Inserts or replaces the volatile row for `(s, key)`. -/
  volatilePut : VolatileRow → m Unit
  /-- Deletes the volatile row for `(s, key)` if present. -/
  volatileDel : (s key : Int64) → m Unit
  /-- Adds a predicate to the multi-eid set; idempotent. -/
  addPredMulti : Int64 → m Unit
  /-- Begins a write transaction (`BEGIN IMMEDIATE` on SQLite). -/
  begin : m Unit
  /-- Commits the write transaction. -/
  commit : m Unit
  /-- Rolls the write transaction back. -/
  rollback : m Unit
  /-- Opens a savepoint. -/
  savepoint : String → m Unit
  /-- Restores the state at the newest open savepoint of that name and keeps it open. -/
  rollbackTo : String → m Unit
  /-- Closes the newest open savepoint of that name and every newer one, keeping writes. -/
  release : String → m Unit

/--
Snapshot readers: separate handles that observe the committed state as of `beginRead`.
`withSnapshot` runs reads (a `ReadStore r` program) on a snapshot.
-/
class SnapshotStore (m : Type → Type) (snap : outParam Type) (r : outParam (Type → Type)) where
  beginRead : m snap
  endRead : snap → m Unit
  withSnapshot {α : Type} : snap → r α → m α

end Tiramemsu.Store
