/-
SQLite connections through leansqlite: opening writer and reader connections configured like
the Rust build, prepared-statement reuse, value binding and reading, transactions,
savepoints and read snapshots. Shell module (unverified).

Only `SQLite.FFI` and `SQLite.LowLevel` are imported: the higher layers of leansqlite import
Lean compiler modules, which the runtime import closure excludes.
-/
import SQLite.LowLevel
import Std.Data.HashMap
import Tiramemsu.Store.Types

namespace Tiramemsu.Sqlite

open Tiramemsu.Store

-- @lat: [[architecture#SQLite Boundary]]

/-- `SQLITE_OPEN_*` flags (sqlite3.h). -/
def openReadOnly : Int32 := 0x00000001
def openReadWrite : Int32 := 0x00000002
def openCreate : Int32 := 0x00000004
/-- Extended result codes on every call of the connection (SQLite ≥ 3.37). -/
def openExResCode : Int32 := 0x02000000

/-- Primary result codes used by the store. -/
def SQLITE_CONSTRAINT : UInt32 := 19
def SQLITE_BUSY : UInt32 := 5
/-- Extended constraint result codes. -/
def SQLITE_CONSTRAINT_TRIGGER : UInt32 := 1811
def SQLITE_CONSTRAINT_UNIQUE : UInt32 := 2067
def SQLITE_CONSTRAINT_PRIMARYKEY : UInt32 := 1555

/-- The monad of SQLite calls: IO with typed store errors. -/
abbrev SqlM := ExceptT StoreError IO

/-- A leansqlite failure as a store error. Connections are opened with extended result codes,
so the code leansqlite reports is the extended code; its low byte is the primary code. -/
def toStoreError : IO.Error → StoreError
  | .otherError code msg => .sqlite (code &&& 0xFF) code msg
  | e => .sqlite 1 1 (toString e)

/-- Runs a leansqlite call, mapping its failure. -/
def lift {α : Type} (x : IO α) : SqlM α := do
  match ← (x.toBaseIO) with
  | .ok a => pure a
  | .error e => throw (toStoreError e)

/-- A bound parameter. Text is bound as its UTF-8 bytes and cast in SQL, so embedded NUL
characters survive (leansqlite binds text NUL-terminated). -/
inductive Val where
  | int (v : Int64)
  | null
  | text (s : String)
  | real (bits : UInt64)
  deriving Repr, Inhabited

def Val.ofOpt : Option Int64 → Val
  | some v => .int v
  | none => .null

def Val.ofOptText : Option String → Val
  | some s => .text s
  | none => .null

def Val.ofOptReal : Option UInt64 → Val
  | some b => .real b
  | none => .null

/-- An open connection with its prepared-statement cache. -/
structure Conn where
  db : SQLite
  stmts : IO.Ref (Std.HashMap String SQLite.Stmt)
  readOnly : Bool

/-- Connection settings. -/
structure ConnOptions where
  busyTimeoutMs : Int32 := 5000
  deriving Repr, Inhabited

/-- Executes SQL without parameters or results. -/
def Conn.exec (c : Conn) (sql : String) : SqlM Unit := lift (c.db.exec sql)

private def openRaw (path : System.FilePath) (flags : Int32) (opts : ConnOptions) (readOnly : Bool) :
    SqlM Conn := do
  let raw ← lift (SQLite.FFI.openV2 path.toString (flags ||| openExResCode) "")
  let db : SQLite := { filename := path, connection := raw }
  if opts.busyTimeoutMs > 0 then lift (db.busyTimeout opts.busyTimeoutMs)
  let stmts ← IO.mkRef {}
  pure { db, stmts, readOnly }

/-- The writer connection: read-write, created if missing, WAL, `synchronous = NORMAL`, busy
timeout, and Rust's `recursive_triggers = ON`. -/
def openWriter (path : System.FilePath) (opts : ConnOptions := {}) : SqlM Conn := do
  let c ← openRaw path (openReadWrite ||| openCreate) opts false
  c.exec "PRAGMA journal_mode = WAL"
  c.exec "PRAGMA synchronous = NORMAL"
  c.exec "PRAGMA recursive_triggers = ON"
  pure c

/-- A reader connection: read-only on the same file. -/
def openReader (path : System.FilePath) (opts : ConnOptions := {}) : SqlM Conn := do
  let c ← openRaw path openReadOnly opts true
  c.exec "PRAGMA recursive_triggers = ON"
  pure c

/-- The cached prepared statement for this SQL text, prepared on first use. -/
def Conn.prepare (c : Conn) (sql : String) : SqlM SQLite.Stmt := do
  match (← c.stmts.get)[sql]? with
  | some st => pure st
  | none =>
    let st ← lift (c.db.prepare sql)
    c.stmts.modify (·.insert sql st)
    pure st

/-- Drops every cached statement (before closing a connection). -/
def Conn.clearCache (c : Conn) : IO Unit := c.stmts.set {}

def bindVal (st : SQLite.Stmt) (i : Int32) : Val → SqlM Unit
  | .int v => lift (st.bindInt64 i v)
  | .null => lift (st.bindNull i)
  | .text s => lift (st.bindBlob i s.toUTF8)
  | .real b => lift (st.bindFloat i (Float.ofBits b))

/-- Runs a cached statement with parameters; the statement is reset afterwards in every case. -/
def Conn.withStmt {α : Type} (c : Conn) (sql : String) (params : Array Val)
    (k : SQLite.Stmt → SqlM α) : SqlM α := do
  let st ← c.prepare sql
  try
    for h : i in [0:params.size] do
      bindVal st (i + 1).toInt32 params[i]
    k st
  finally
    -- `sqlite3_reset` repeats the error of a failed step; that error was already reported.
    let _ ← (st.reset.toBaseIO : BaseIO _)

/-- Steps once; `true` when a row is available. -/
def step (st : SQLite.Stmt) : SqlM Bool := lift st.step

/-- Executes a statement that returns no rows. -/
def Conn.run (c : Conn) (sql : String) (params : Array Val := #[]) : SqlM Unit :=
  c.withStmt sql params fun st => do let _ ← step st

/-- Rows changed by the last statement. -/
def Conn.changes (c : Conn) : SqlM Int64 := lift c.db.changes

/-- An integer column; `none` for NULL. -/
def colInt (st : SQLite.Stmt) (i : Int32) : SqlM (Option Int64) := do
  if (← lift (st.columnType i)) == .null then pure none
  else some <$> lift (st.columnInt64 i)

/-- An integer column that must not be NULL. -/
def colInt! (st : SQLite.Stmt) (i : Int32) : SqlM Int64 := do
  match ← colInt st i with
  | some v => pure v
  | none => throw (.sqlite 1 1 s!"unexpected NULL in column {i}")

/-- A text column read as bytes (the SQL must select `CAST(col AS BLOB)`); `none` for NULL. -/
def colText (st : SQLite.Stmt) (i : Int32) : SqlM (Option String) := do
  if (← lift (st.columnType i)) == .null then pure none
  else
    let bytes ← lift (st.columnBlob i)
    match String.fromUTF8? bytes with
    | some s => pure (some s)
    | none => throw (.sqlite 1 1 s!"invalid UTF-8 in column {i}")

/-- A REAL column as IEEE bits; `none` for NULL. -/
def colReal (st : SQLite.Stmt) (i : Int32) : SqlM (Option UInt64) := do
  if (← lift (st.columnType i)) == .null then pure none
  else (some ·.toBits) <$> lift (st.columnDouble i)

/-- Collects every row of a statement. -/
partial def collectRows {α : Type} (st : SQLite.Stmt) (row : SQLite.Stmt → SqlM α) : SqlM (Array α) := do
  let rec go (acc : Array α) : SqlM (Array α) := do
    if ← step st then go (acc.push (← row st)) else pure acc
  go #[]

/-- The first row of a query, if any. -/
def Conn.queryOne {α : Type} (c : Conn) (sql : String) (params : Array Val)
    (row : SQLite.Stmt → SqlM α) : SqlM (Option α) :=
  c.withStmt sql params fun st => do
    if ← step st then some <$> row st else pure none

/-- All rows of a query. -/
def Conn.queryAll {α : Type} (c : Conn) (sql : String) (params : Array Val)
    (row : SQLite.Stmt → SqlM α) : SqlM (Array α) :=
  c.withStmt sql params fun st => collectRows st row

/-! ## Transactions, savepoints, snapshots -/

/-- A savepoint name as a quoted SQL identifier. -/
def quoteIdent (n : String) : String := "\"" ++ n.replace "\"" "\"\"" ++ "\""

def Conn.beginImmediate (c : Conn) : SqlM Unit := c.exec "BEGIN IMMEDIATE"
def Conn.commit (c : Conn) : SqlM Unit := c.exec "COMMIT"
def Conn.rollback (c : Conn) : SqlM Unit := c.exec "ROLLBACK"
def Conn.savepoint (c : Conn) (n : String) : SqlM Unit := c.exec s!"SAVEPOINT {quoteIdent n}"
def Conn.rollbackTo (c : Conn) (n : String) : SqlM Unit :=
  c.exec s!"ROLLBACK TO SAVEPOINT {quoteIdent n}"
def Conn.release (c : Conn) (n : String) : SqlM Unit := c.exec s!"RELEASE SAVEPOINT {quoteIdent n}"

/-- Whether the connection is inside a transaction. -/
def Conn.inTransaction (c : Conn) : SqlM Bool := lift c.db.inTransaction

/-- Begins a read transaction and pins its WAL snapshot with an immediate read of `meta`:
a deferred transaction takes its snapshot only at the first read. -/
def Conn.beginRead (c : Conn) : SqlM Unit := do
  c.exec "BEGIN"
  try
    let _ ← c.queryOne "SELECT count(*) FROM meta" #[] fun st => colInt st 0
  catch e =>
    let _ ← (c.rollback.run : IO _)
    throw e

/-- Ends a read transaction. -/
def Conn.endRead (c : Conn) : SqlM Unit := c.exec "COMMIT"

/-! ## Library facts -/

/-- The linked SQLite version (`sqlite_version()`). -/
def Conn.sqliteVersion (c : Conn) : SqlM String := do
  let v ← c.queryOne "SELECT CAST(sqlite_version() AS BLOB)" #[] fun st => colText st 0
  pure ((v.bind id).getD "")

/-- `PRAGMA compile_options` of the linked SQLite. -/
def Conn.compileOptions (c : Conn) : SqlM (Array String) := do
  let rows ← c.queryAll "SELECT CAST(compile_options AS BLOB) FROM pragma_compile_options" #[]
    fun st => colText st 0
  pure (rows.filterMap id)

/-- Opens an in-memory connection (for version reporting and probes). -/
def openMemory : SqlM Conn := openRaw ":memory:" (openReadWrite ||| openCreate) {} false

end Tiramemsu.Sqlite
