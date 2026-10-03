// @tiramemsu/node — a bitemporal, never-forget triple store on SQLite for Node.js.
//
// The native addon (src/lib.rs) exchanges JSON strings with tiramemsu-json; this
// wrapper gives those calls typed, idiomatic names. See README.md for usage.

import { createRequire } from "node:module";

// ---- Native addon -------------------------------------------------------

interface NativeInstance {
  call(op: string, args: string): string;
}
interface NativeModule {
  Native: new (path: string, options?: string | null) => NativeInstance;
}

function loadNative(): NativeModule {
  const require = createRequire(import.meta.url);
  const target = `${process.platform}-${process.arch}`;
  for (const file of [`../tiramemsu.${target}.node`, "../tiramemsu.node"]) {
    try {
      return require(file) as NativeModule;
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== "MODULE_NOT_FOUND") throw e;
    }
  }
  throw new Error(
    `@tiramemsu/node has no prebuilt binary for ${target}; ` +
      `build it with \`npm run build:native\` in a checkout of https://github.com/Volland/tiramemsu`,
  );
}

const _native = loadNative();

// ---- Terms ---------------------------------------------------------------

/** A named node (IRI). */
export type IriTerm = { readonly iri: string };
/** An anonymous node (internal id, assigned by the store). */
export type NodeTerm = { readonly node: number };
/** A blank node. */
export type BNodeTerm = { readonly bnode: number };
/** A reference to a statement by its eid. */
export type StmtTerm = { readonly stmt: number };
/** A reference to a transaction by its id. */
export type TxTerm = { readonly tx: number };
/** A typed or language-tagged literal. */
export type LiteralTerm = { readonly lex: string; readonly datatype?: string; readonly lang?: string };

/**
 * A decoded term from the database. Plain strings, numbers and booleans are returned
 * as-is. Integers beyond 2^53 are returned as `bigint` (bridge form `{"$int":"..."}`), and an
 * `xsd:dateTime` is returned as a `Date`.
 */
export type Term =
  | string
  | number
  | boolean
  | bigint
  | Date
  | IriTerm
  | NodeTerm
  | BNodeTerm
  | StmtTerm
  | TxTerm
  | LiteralTerm;

/**
 * A value accepted as a term argument. Accepts all Term values, plus:
 * - `Date` → xsd:dateTime literal
 * - `bigint` → `{"$int":"…"}`
 * - `Ref` → a named handle from an earlier op in the same transaction
 */
export type TermInput = Term | Date | Ref;

// ---- Term helpers --------------------------------------------------------

/** Constructs an IRI term. */
export function iri(s: string): IriTerm { return { iri: s }; }
/** Constructs a typed or language literal. */
export function literal(lex: string, opts?: { datatype?: string; lang?: string }): LiteralTerm {
  return opts ? { lex, ...opts } : { lex };
}
/** Constructs an anonymous-node term (internal id). */
export function node(n: number): NodeTerm { return { node: n }; }
/** Constructs a blank-node term. */
export function bnode(n: number): BNodeTerm { return { bnode: n }; }
/** Constructs a statement-reference term. */
export function stmt(eid: number): StmtTerm { return { stmt: eid }; }
/** Constructs a transaction-reference term. */
export function tx(n: number): TxTerm { return { tx: n }; }

/**
 * A handle returned by `assert`, `create`, `supersede` and `newNode` inside a
 * transaction callback. Pass it as `s`, `o` or `eid` to a later op in the same
 * callback; the bridge resolves it to the actual eid at commit time.
 */
export class Ref {
  /** @internal */ constructor(/** @internal */ readonly _refName: string) {}
}

function isRef(t: unknown): t is Ref { return t instanceof Ref; }

function toJson(t: TermInput): unknown {
  if (t instanceof Date) return { lex: t.toISOString(), datatype: "http://www.w3.org/2001/XMLSchema#dateTime" };
  if (typeof t === "bigint") return { $int: t.toString() };
  if (isRef(t)) return { ref: t._refName };
  return t; // string | number | boolean | IriTerm | NodeTerm | ...
}

function fromJson(j: unknown): Term {
  if (typeof j === "string" || typeof j === "number" || typeof j === "boolean") return j;
  if (j !== null && typeof j === "object" && "$int" in j) {
    return BigInt((j as { $int: string }).$int);
  }
  if (
    j !== null &&
    typeof j === "object" &&
    (j as { datatype?: string }).datatype === "http://www.w3.org/2001/XMLSchema#dateTime" &&
    typeof (j as { lex?: unknown }).lex === "string"
  ) {
    const d = new Date((j as { lex: string }).lex);
    if (!Number.isNaN(d.getTime())) return d;
  }
  return j as Term;
}

/** Converts a time argument to the form the bridge accepts (epoch-ms or RFC 3339 string). */
function timeArg(t: Date | number | string): number | string {
  return t instanceof Date ? t.getTime() : t;
}

// ---- Error ---------------------------------------------------------------

/**
 * An error from a tiramemsu operation. The `code` field is a machine-readable string,
 * e.g. `Parse`, `NotLive`, `InvalidArgument`, `Unsupported`. See README for the full list.
 */
export class TiramemsuError extends Error {
  constructor(message: string, public readonly code: string) {
    super(message);
    this.name = "TiramemsuError";
  }
}

function parseError(e: unknown): TiramemsuError {
  const msg = e instanceof Error ? e.message : String(e);
  const prefix = "tiramemsu:";
  if (msg.startsWith(prefix)) {
    try {
      const obj = JSON.parse(msg.slice(prefix.length)) as { code: string; message: string };
      return new TiramemsuError(obj.message, obj.code);
    } catch {
      // fall through
    }
  }
  return new TiramemsuError(msg, "Error");
}

function callNative(db: NativeInstance, op: string, args: unknown): unknown {
  try {
    return JSON.parse(db.call(op, JSON.stringify(args)));
  } catch (e) {
    throw parseError(e);
  }
}

// ---- Result types --------------------------------------------------------

/** One SELECT row: variable name → decoded Term. */
export type SelectRow = Record<string, Term>;

/** A discriminated SPARQL result. */
export type SparqlResult =
  | {
      kind: "select";
      vars: string[];
      rows: SelectRow[];
      /** With `{ provenance: true }`: the statements behind each row, parallel to `rows`. */
      provenance?: StmtTerm[][];
    }
  | { kind: "ask"; value: boolean }
  | { kind: "graph"; triples: Array<{ s: Term; p: Term; o: Term }> }
  | { kind: "update"; report: Report };

/** A Cypher read result. */
export interface CypherResult {
  columns: string[];
  rows: unknown[][];
}

/** One statement row with bitemporal metadata. */
export interface TripleRow {
  eid: number;
  s: Term;
  p: Term;
  o: Term;
  tAdd: number;
  tRet: number | null;
  validFrom: number | null;
  validTo: number | null;
  retKind: "explicit" | "cascade" | "supersede" | "cardinality" | null;
}

/** A path search result row. */
export interface PathRow {
  start: Term;
  end: Term;
  hops: number;
  path: {
    nodes: Term[];
    hops: Array<{ eid: Term; predicate: Term; dir: "out" | "in"; kind: string }>;
  } | null;
  /** Arrival instant (epoch ms) of a time-respecting search, otherwise `null`. */
  arrival: number | null;
}

/** Options for `View.path`. */
export interface PathOptions {
  mode?: "reach" | "trail" | "anyShortest" | "allShortest";
  maxHops?: number;
  /** Only hops whose statements belong to one of these graphs (a term that is not stored names no graph). */
  graphs?: TermInput[];
  /** Earliest-arrival evaluation: each hop must start no earlier than the previous one, optionally after `after`. */
  timeRespecting?: boolean | { after?: Date | number | string };
}

/** A `tiramemsu-bundle/1` JSON document: a statement with its layers and evidence. */
export type Bundle = Record<string, unknown>;

/** The result of an `importBundle` op. */
export interface ImportedBundle {
  root: number;
  statements: Array<{ id: string; eid: number; new: boolean }>;
}

/** An event log entry. */
export interface EventRow {
  t: number;
  eid: number;
  op: "assert" | "retract";
  kind: string | null;
}

/** A retracted statement in a report. */
export interface RetractedRecord { eid: number; kind: string }
/** A superseded pair in a report. */
export interface SupersededRecord { old: number; new: number }

/** The result of a committed (or dry-run) transaction. */
export interface Report {
  t: number;
  instant: number;
  asserted: number[];
  existing: number[];
  retracted: RetractedRecord[];
  superseded: SupersededRecord[];
  memberships: number[];
  membershipsRetracted: RetractedRecord[];
  /** One result object per op, in order. */
  results: unknown[];
  /** Named refs from `as` labels, mapping name → eid. */
  refs: Record<string, number>;
}

// ---- Decode helpers ------------------------------------------------------

function decodeSparql(r: Record<string, unknown>): SparqlResult {
  if (r.kind === "select") {
    const rows = (r.rows as Record<string, unknown>[]).map((row) => {
      const out: SelectRow = {};
      for (const [k, v] of Object.entries(row)) out[k] = fromJson(v);
      return out;
    });
    const out: SparqlResult = { kind: "select", vars: r.vars as string[], rows };
    if (r.provenance) out.provenance = r.provenance as StmtTerm[][];
    return out;
  }
  if (r.kind === "ask") return { kind: "ask", value: r.value as boolean };
  if (r.kind === "graph") {
    const triples = (r.triples as Array<Record<string, unknown>>).map((t) => ({
      s: fromJson(t.s), p: fromJson(t.p), o: fromJson(t.o),
    }));
    return { kind: "graph", triples };
  }
  if (r.kind === "update") return { kind: "update", report: r.report as Report };
  throw new TiramemsuError(`unknown SPARQL result kind: ${String(r.kind)}`, "Error");
}

function decodeTriple(t: Record<string, unknown>): TripleRow {
  return {
    eid: t.eid as number,
    s: fromJson(t.s), p: fromJson(t.p), o: fromJson(t.o),
    tAdd: t.tAdd as number,
    tRet: t.tRet as number | null,
    validFrom: t.validFrom as number | null,
    validTo: t.validTo as number | null,
    retKind: (t.retKind ?? null) as TripleRow["retKind"],
  };
}

// ---- View ----------------------------------------------------------------

/** Options for opening a database. */
export interface OpenOptions {
  readers?: number;
  busyTimeoutMs?: number;
  termCacheCapacity?: number;
  optimizeEvery?: number;
  pathMaxHops?: number;
  pathMaxStates?: number;
  /** How long a read waits for a free reader before failing with `PoolTimeout` (default: no limit). */
  readerTimeoutMs?: number;
}

/**
 * The bounds of one call, for `View.withBudget` and the `budget` transaction option.
 * Every field is optional and independent; an empty budget bounds nothing.
 * Failures are `TiramemsuError`s with code `DeadlineExceeded`, `Cancelled`,
 * `PoolTimeout` or `ResultLimitExceeded`; a stopped write commits nothing.
 */
export interface QueryBudget {
  /** The longest the whole call may take. */
  timeoutMs?: number;
  /** How long to wait for a free reader (overrides `OpenOptions.readerTimeoutMs`). */
  readerTimeoutMs?: number;
  /** The most rows the call may decode, across every statement it runs. */
  maxRows?: number;
  /** The most decoded result bytes the call may produce. */
  maxBytes?: number;
  /** A key that `Database.cancel(key)` uses to stop the call. */
  cancelKey?: string;
}

/** A time reference for `Database.asOf`. */
export type TimeRef = { tx: number } | { instant: Date | number | string };

type ViewJson = { kind: string; tx?: number; instant?: number | string; validAt?: number | string };

/**
 * An immutable view of the database at a point in transaction time, optionally
 * filtered by valid time. All queries run through a view.
 */
export class View {
  /** @internal */ constructor(
    private readonly _db: NativeInstance,
    private readonly _view: ViewJson,
    private readonly _budget?: QueryBudget,
  ) {}

  /** Returns a new View narrowed to facts valid at `t`. */
  validAt(t: Date | number | string): View {
    return new View(this._db, { ...this._view, validAt: timeArg(t) }, this._budget);
  }

  /**
   * Returns the same view with every call through it bounded by `budget`: each call
   * (`sparql`, `cypher`, `triples`, `path`, ...) is one operation with its own deadline
   * and its own row and byte budget.
   */
  withBudget(budget: QueryBudget): View {
    return new View(this._db, this._view, budget);
  }

  /** @internal The view (and budget) every read sends. */
  private _base(): Record<string, unknown> {
    return this._budget ? { view: this._view, budget: this._budget } : { view: this._view };
  }

  /**
   * SPARQL query (SELECT, ASK, CONSTRUCT, DESCRIBE, or UPDATE). With `provenance`, a SELECT
   * result also lists the statements that produced each row.
   */
  sparql(text: string, opts?: { provenance?: boolean }): SparqlResult {
    const args: Record<string, unknown> = { ...this._base(), text };
    if (opts?.provenance) args.provenance = true;
    return decodeSparql(callNative(this._db, "sparql", args) as Record<string, unknown>);
  }

  /** openCypher read query. */
  cypher(text: string, params?: Record<string, unknown>): CypherResult {
    const r = callNative(this._db, "cypher", { ...this._base(), text, params }) as Record<string, unknown>;
    return { columns: r.columns as string[], rows: r.rows as unknown[][] };
  }

  /** Pattern match on statements; each of s, p, o is optional (absent = any). */
  triples(pattern?: { s?: TermInput; p?: TermInput; o?: TermInput }): TripleRow[] {
    const args: Record<string, unknown> = this._base();
    if (pattern?.s !== undefined) args.s = toJson(pattern.s);
    if (pattern?.p !== undefined) args.p = toJson(pattern.p);
    if (pattern?.o !== undefined) args.o = toJson(pattern.o);
    return (callNative(this._db, "triples", args) as Record<string, unknown>[]).map(decodeTriple);
  }

  /** Reachability or shortest-path search. `pathExpr` is a predicate path expression. */
  path(
    start: TermInput,
    pathExpr: string,
    opts?: PathOptions,
  ): PathRow[] {
    const args: Record<string, unknown> = { ...this._base(), start: toJson(start), path: pathExpr };
    if (opts?.mode) args.mode = opts.mode;
    if (opts?.maxHops !== undefined) args.maxHops = opts.maxHops;
    if (opts?.graphs) args.graphs = opts.graphs.map(toJson);
    const tr = opts?.timeRespecting;
    if (tr === true) args.timeRespecting = true;
    else if (tr && typeof tr === "object") {
      args.timeRespecting = tr.after !== undefined ? { after: timeArg(tr.after) } : {};
    }
    return (callNative(this._db, "path", args) as Record<string, unknown>[]).map((r) => ({
      start: fromJson(r.start),
      end: fromJson(r.end),
      hops: r.hops as number,
      path: r.path
        ? {
            nodes: ((r.path as { nodes: unknown[] }).nodes).map(fromJson),
            hops: ((r.path as { hops: Array<Record<string, unknown>> }).hops).map((h) => ({
              eid: fromJson(h.eid),
              predicate: fromJson(h.predicate),
              dir: h.dir as "out" | "in",
              kind: h.kind as string,
            })),
          }
        : null,
      arrival: (r.arrival ?? null) as number | null,
    }));
  }

  /** Events since transaction `since` (default: 0 = all). */
  events(since?: number): EventRow[] {
    return callNative(this._db, "events", { ...this._base(), since: since ?? 0 }) as EventRow[];
  }

  /** Named graphs known in this view. */
  graphs(): Term[] {
    return (callNative(this._db, "graphs", this._base()) as unknown[]).map(fromJson);
  }

  /** Statement eids that belong to graph `g`. */
  graphMembers(g: TermInput): number[] {
    return callNative(this._db, "graphMembers", { ...this._base(), graph: toJson(g) }) as number[];
  }

  /** Statements that stand on `eid` (its layers and memberships, transitively): what a retraction would cascade to. */
  dependents(eid: number | StmtTerm): number[] {
    return callNative(this._db, "dependents", { ...this._base(), eid }) as number[];
  }

  /** The statement `eid` with its layers and evidence, as `tiramemsu-bundle/1` JSON for `Tx.importBundle`. */
  bundle(eid: number | StmtTerm): Bundle {
    return callNative(this._db, "bundle", { ...this._base(), eid }) as Bundle;
  }

  /** All object values for a given subject and predicate. */
  values(s: TermInput, key: TermInput): Term[] {
    return (
      callNative(this._db, "values", { ...this._base(), s: toJson(s), key: toJson(key) }) as unknown[]
    ).map(fromJson);
  }
}

// ---- Transaction ---------------------------------------------------------

/** Options for assert and create. */
export interface AssertOpts {
  validFrom?: Date | number | string;
  validTo?: Date | number | string;
  onExisting?: "return" | "confirm";
}

/** Options for a transaction. */
export interface TxOptions {
  dryRun?: boolean;
  maxCascade?: number;
  /** Bounds the transaction; a stopped one commits nothing. */
  budget?: QueryBudget;
}

/** Splits the `budget` off transaction options into the call arguments. */
function txArgs(args: Record<string, unknown>, options?: TxOptions): Record<string, unknown> {
  if (!options) return args;
  const { budget, ...rest } = options;
  if (Object.keys(rest).length > 0) args.options = rest;
  if (budget) args.budget = budget;
  return args;
}

/** Database metadata returned by `Database.info()`. */
export interface DatabaseInfo {
  path: string;
  readers: number;
  pathMaxHops: number;
  importActive: boolean;
  statisticsDue: boolean;
}

/** The result of a speculate call. */
export interface SpeculateResult {
  results: unknown[];
}

let _refSeq = 0;

/**
 * Transaction op accumulator passed to `Database.transact`. Ops are recorded but not
 * sent to the database until the callback returns; `Ref` handles resolve at commit time.
 */
export class Tx {
  /** @internal */ readonly _ops: unknown[] = [];

  private _push(op: Record<string, unknown>, named: boolean): Ref {
    const ref = new Ref(named ? `r${++_refSeq}` : "_");
    if (named) op.as = ref._refName;
    this._ops.push(op);
    return ref;
  }

  /** Asserts (s, p, o), idempotent if already live. Returns a Ref to the statement eid. */
  assert(s: TermInput, p: TermInput, o: TermInput, opts?: AssertOpts): Ref {
    const op: Record<string, unknown> = { op: "assert", s: toJson(s), p: toJson(p), o: toJson(o) };
    if (opts?.validFrom !== undefined) op.validFrom = timeArg(opts.validFrom);
    if (opts?.validTo !== undefined) op.validTo = timeArg(opts.validTo);
    if (opts?.onExisting) op.onExisting = opts.onExisting;
    return this._push(op, true);
  }

  /** Creates a statement, failing if an identical live statement already exists. */
  create(s: TermInput, p: TermInput, o: TermInput, opts?: AssertOpts): Ref {
    const op: Record<string, unknown> = { op: "create", s: toJson(s), p: toJson(p), o: toJson(o) };
    if (opts?.validFrom !== undefined) op.validFrom = timeArg(opts.validFrom);
    if (opts?.validTo !== undefined) op.validTo = timeArg(opts.validTo);
    return this._push(op, true);
  }

  /** Retracts the statement with the given eid (and cascades to its layers). */
  retract(eid: number | Ref): void {
    this._ops.push({ op: "retract", eid: isRef(eid) ? { ref: (eid as Ref)._refName } : eid });
  }

  /** Retracts all statements matching the pattern. */
  retractMatching(pattern: { s?: TermInput; p?: TermInput; o?: TermInput }): void {
    const op: Record<string, unknown> = { op: "retractMatching" };
    if (pattern.s !== undefined) op.s = toJson(pattern.s);
    if (pattern.p !== undefined) op.p = toJson(pattern.p);
    if (pattern.o !== undefined) op.o = toJson(pattern.o);
    this._ops.push(op);
  }

  /** Corrects a statement (valid interval or object), replaying its layers onto the new row. */
  supersede(
    eid: number | Ref,
    patch: { o?: TermInput; validFrom?: Date | number | string; validTo?: Date | number | string },
  ): Ref {
    const p: Record<string, unknown> = {};
    if (patch.o !== undefined) p.o = toJson(patch.o);
    if (patch.validFrom !== undefined) p.validFrom = timeArg(patch.validFrom);
    if (patch.validTo !== undefined) p.validTo = timeArg(patch.validTo);
    return this._push({
      op: "supersede",
      eid: isRef(eid) ? { ref: (eid as Ref)._refName } : eid,
      patch: p,
    }, true);
  }

  /** Records that another source confirms the statement without changing it. */
  confirm(eid: number | Ref): void {
    this._ops.push({ op: "confirm", eid: isRef(eid) ? { ref: (eid as Ref)._refName } : eid });
  }

  /** Asserts metadata on the transaction itself (subject is the tx node). */
  meta(p: TermInput, o: TermInput): void {
    this._ops.push({ op: "meta", p: toJson(p), o: toJson(o) });
  }

  /** Upsert: assert if no live statement with `p` exists, otherwise confirm. */
  upsert(p: TermInput, o: TermInput): void {
    this._ops.push({ op: "upsert", p: toJson(p), o: toJson(o) });
  }

  /** Allocates a new anonymous node id and returns its Ref. */
  newNode(): Ref {
    return this._push({ op: "newNode" }, true);
  }

  /** Tags the statement `eid` with membership in named graph `graph`. */
  addToGraph(eid: number | Ref, graph: TermInput, opts?: Pick<AssertOpts, "onExisting">): Ref {
    return this._push({
      op: "addToGraph",
      eid: isRef(eid) ? { ref: (eid as Ref)._refName } : eid,
      graph: toJson(graph),
      ...(opts?.onExisting ? { onExisting: opts.onExisting } : {}),
    }, true);
  }

  /** Removes the membership of `eid` from `graph`. */
  removeFromGraph(eid: number | Ref, graph: TermInput): void {
    this._ops.push({
      op: "removeFromGraph",
      eid: isRef(eid) ? { ref: (eid as Ref)._refName } : eid,
      graph: toJson(graph),
    });
  }

  /** Removes all memberships from `graph`. */
  clearGraph(graph: TermInput): void {
    this._ops.push({ op: "clearGraph", graph: toJson(graph) });
  }

  /** Creates a named graph. */
  createGraph(graph: TermInput): Ref {
    return this._push({ op: "createGraph", graph: toJson(graph) }, true);
  }

  /** Drops a named graph and all its memberships. */
  dropGraph(graph: TermInput): void {
    this._ops.push({ op: "dropGraph", graph: toJson(graph) });
  }

  /**
   * Imports a bundle read with `View.bundle` (possibly from another database). The Ref names
   * the imported root statement; the op result is an `ImportedBundle`.
   */
  importBundle(bundle: Bundle): Ref {
    return this._push({ op: "importBundle", bundle }, true);
  }

  /** Runs a Cypher write statement inside this transaction. */
  cypher(text: string, params?: Record<string, unknown>): void {
    this._ops.push({ op: "cypher", text, params });
  }
}

// ---- Bulk import --------------------------------------------------------

/** What a bulk import session has done so far. */
export interface ImportProgress {
  /** Chunks that committed. */
  chunks: number;
  /** Chunks that failed and were rolled back (they left no trace). */
  rejected: number;
  /** New statements committed. */
  asserted: number;
  /** Assertions that reused a live statement. */
  existing: number;
  /** Statements retracted. */
  retracted: number;
  /** Transaction number of every committed chunk. */
  txs: number[];
  /** Time spent running chunks. */
  elapsedMs: number;
  /** Time spent in the final statistics refresh. */
  maintenanceMs: number;
}

/** The result of one committed chunk: the transaction report plus the session progress. */
export interface ChunkReport extends Report {
  progress: ImportProgress;
}

/**
 * The outcome of `BulkImport.finish()`. Every chunk in `progress.txs` is committed even
 * when `maintenanceError` is set: a failed final analysis never rolls data back.
 */
export interface ImportSummary {
  progress: ImportProgress;
  analyzed: boolean;
  statisticsDue: boolean;
  maintenanceError: { code: string; message: string } | null;
}

/**
 * A bulk import session from `Database.bulkImport()`. Each chunk is one atomic
 * transaction; statistics are refreshed once by `finish()`. While the session is open,
 * other writes fail with `ImportInProgress`; reads see the last committed chunk. Always
 * end it with `finish()` or `cancel()` (use `try/finally`): the lease is held until then.
 */
export class BulkImport {
  private readonly _db: NativeInstance;
  /** The bridge session id. */
  readonly session: number;

  /** @internal */
  constructor(db: NativeInstance, session: number) {
    this._db = db;
    this.session = session;
  }

  /** Commits `fn` (or `ops`) as one chunk. A failing chunk throws and rolls back alone. */
  chunk(fn: (tx: Tx) => void, options?: TxOptions): ChunkReport;
  chunk(ops: unknown[], options?: TxOptions): ChunkReport;
  chunk(fnOrOps: ((tx: Tx) => void) | unknown[], options?: TxOptions): ChunkReport {
    let ops: unknown[];
    if (typeof fnOrOps === "function") {
      const t = new Tx();
      fnOrOps(t);
      ops = t._ops;
    } else {
      ops = fnOrOps;
    }
    return callNative(this._db, "importChunk", txArgs({ session: this.session, ops }, options)) as ChunkReport;
  }

  /** The progress so far. */
  progress(): ImportProgress {
    return callNative(this._db, "importProgress", { session: this.session }) as ImportProgress;
  }

  /** Runs one full analysis, releases the lease and returns the summary. */
  finish(): ImportSummary {
    return callNative(this._db, "importFinish", { session: this.session }) as ImportSummary;
  }

  /** Ends the session without analysis; committed chunks stay and statistics are left due. */
  cancel(): ImportProgress {
    return callNative(this._db, "importCancel", { session: this.session }) as ImportProgress;
  }
}

// ---- Database -----------------------------------------------------------

/**
 * An open tiramemsu database. All operations are synchronous.
 * The underlying SQLite file is held open until the object is garbage-collected.
 */
export class Database {
  private readonly _db: NativeInstance;

  private constructor(db: NativeInstance) { this._db = db; }

  /**
   * Opens (creating if needed) the database at `path`.
   * `options` controls reader count, busy timeout and other limits.
   */
  static open(path: string, options?: OpenOptions): Database {
    const optStr =
      options && Object.keys(options).length > 0 ? JSON.stringify(options) : undefined;
    return new Database(new _native.Native(path, optStr ?? null));
  }

  /** A view of the live database (no valid-time filter). */
  now(): View { return new View(this._db, { kind: "now" }); }

  /** A view as of a past transaction number (`{tx: n}`) or wall-clock instant (`{instant: ...}`). */
  asOf(ref: TimeRef): View {
    if ("tx" in ref) return new View(this._db, { kind: "asOf", tx: ref.tx });
    return new View(this._db, { kind: "asOf", instant: timeArg(ref.instant) });
  }

  /** A view of all statements ever, including retracted ones. */
  history(): View { return new View(this._db, { kind: "history" }); }

  /** Runs `fn` (or applies `ops`) as one atomic transaction. */
  transact(fn: (tx: Tx) => void, options?: TxOptions): Report;
  transact(ops: unknown[], options?: TxOptions): Report;
  transact(fnOrOps: ((tx: Tx) => void) | unknown[], options?: TxOptions): Report {
    let ops: unknown[];
    if (typeof fnOrOps === "function") {
      const t = new Tx();
      fnOrOps(t);
      ops = t._ops;
    } else {
      ops = fnOrOps;
    }
    return callNative(this._db, "transact", txArgs({ ops }, options)) as Report;
  }

  /** A Cypher statement that may write, executed as one transaction. */
  cypherWrite(text: string, params?: Record<string, unknown>, options?: TxOptions): unknown {
    const args: Record<string, unknown> = { text };
    if (params) args.params = params;
    return callNative(this._db, "cypherWrite", txArgs(args, options));
  }

  /**
   * Cancels the call running with `budget.cancelKey === key` (or the next one to start
   * with it). Returns whether a running call held the key. Use a fresh key per call.
   */
  cancel(key: string): boolean {
    return (callNative(this._db, "cancel", { key }) as { running: boolean }).running;
  }

  /**
   * Hypothetically applies `fn`, runs `queries` on the result, and discards the changes.
   * Equivalent to a dry-run + read in one round trip.
   */
  speculate(
    fn: (tx: Tx) => void,
    queries: Array<{ op: string } & Record<string, unknown>>,
  ): SpeculateResult {
    const t = new Tx();
    fn(t);
    return callNative(this._db, "with", { ops: t._ops, queries }) as SpeculateResult;
  }

  /**
   * Starts a bulk import session: chunked atomic writes with planner statistics refreshed
   * once at `finish()`. Throws `ImportInProgress` while another session is open.
   */
  bulkImport(): BulkImport {
    const r = callNative(this._db, "importBegin", {}) as { session: number };
    return new BulkImport(this._db, r.session);
  }

  /** Refreshes the query planner's statistics (run after large imports). */
  optimize(): void { callNative(this._db, "optimize", {}); }

  /**
   * Returns database metadata: file path, reader count, pathMaxHops, whether an import
   * session holds the write lease, and whether planner statistics are due.
   */
  info(): DatabaseInfo {
    return callNative(this._db, "info", {}) as DatabaseInfo;
  }
}
