// @tiramemsu/node — integration tests covering all required scenarios.
// Run after `npm run build:native` (the .node file must exist).

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it, expect } from "vitest";
import { Database, iri, stmt, TiramemsuError } from "../lib/index.ts";

// Helpers matching the bridge test namespace.
const v = (s: string) => iri(`urn:tiramemsu:v:${s}`);
const alice    = v("alice");
const acme     = v("acme");
const globex   = v("globex");
const worksAt  = v("worksAt");
const confidence = v("confidence");
const knows    = v("knows");

function tempDb(): { db: Database; dir: string } {
  const dir = mkdtempSync(join(tmpdir(), "tiramemsu-test-"));
  const db  = Database.open(join(dir, "test.db"));
  return { db, dir };
}

function cleanup(dir: string): void {
  try { rmSync(dir, { recursive: true, force: true }); } catch {}
}

// ---- (a) Alice time-travel story ----------------------------------------

describe("time travel (Alice story)", () => {
  it("covers all four view combinations", () => {
    const { db, dir } = tempDb();
    try {
      // tx1: assert alice worksAt acme valid from 2020-01-01, with a confidence layer
      const r1 = db.transact((tx) => {
        const job = tx.assert(alice, worksAt, acme, { validFrom: "2020-01-01" });
        tx.assert(job, confidence, 0.8);
      });
      const jobEid = (r1.results[0] as { eid: number }).eid;

      // tx2: supersede — she left Acme at end of 2023
      db.transact((tx) => { tx.supersede(jobEid, { validTo: "2024-01-01" }); });

      // tx3: she joins Globex on 2024-03-01
      db.transact((tx) => { tx.assert(alice, worksAt, globex, { validFrom: "2024-03-01" }); });

      const orgs = (v: ReturnType<Database["now"]>): string[] => {
        const r = v.sparql(
          "SELECT ?o WHERE { <urn:tiramemsu:v:alice> <urn:tiramemsu:v:worksAt> ?o }",
        );
        if (r.kind !== "select") throw new Error("expected select");
        return r.rows.map((row) => JSON.stringify(row["o"])).sort();
      };

      // now: both live statements (no valid-time filter)
      const nowOrgs = orgs(db.now());
      expect(nowOrgs).toContain(JSON.stringify(acme));
      expect(nowOrgs).toContain(JSON.stringify(globex));

      // asOf tx=1: only the original Acme statement existed
      expect(orgs(db.asOf({ tx: 1 }))).toEqual([JSON.stringify(acme)]);

      // asOf tx=1, validAt 2026-01-01: we believed she was still there
      expect(orgs(db.asOf({ tx: 1 }).validAt("2026-01-01"))).toEqual([JSON.stringify(acme)]);

      // now, validAt 2026-01-01: today we know it's Globex
      expect(orgs(db.now().validAt("2026-01-01"))).toEqual([JSON.stringify(globex)]);

      // now, validAt 2024-02-01: gap between jobs — nobody
      expect(orgs(db.now().validAt("2024-02-01"))).toHaveLength(0);

      // history: SPARQL sees a set of (s,p,o), so acme appears once despite two episodes
      expect(orgs(db.history()).length).toBe(2); // acme + globex
    } finally { cleanup(dir); }
  });
});

// ---- (b) Layers via Ref -------------------------------------------------

describe("layers via Ref", () => {
  it("asserts a confidence layer on a statement and reads it back", () => {
    const { db, dir } = tempDb();
    try {
      const r = db.transact((tx) => {
        const job = tx.assert(alice, worksAt, acme);
        tx.assert(job, confidence, 0.8);  // job is a Ref → resolved to the first eid
      });

      expect(r.asserted.length).toBe(2);

      // Read back via Cypher: relationship graph
      const cr = db.now().cypher("MATCH (a)-[:worksAt]->(c) RETURN a, c");
      expect(cr.columns).toContain("a");
      expect(cr.rows.length).toBe(1);

      // Read back via SPARQL: the confidence triple sits on the statement's eid
      const sr = db.now().sparql(
        "SELECT ?c WHERE { ?s <urn:tiramemsu:v:confidence> ?c }",
      );
      if (sr.kind === "select") {
        expect(sr.rows.length).toBe(1);
        expect(sr.rows[0]["c"]).toBe(0.8);
      } else { expect.fail("expected select"); }
    } finally { cleanup(dir); }
  });
});

// ---- (c) Retract cascade + events --------------------------------------

describe("retract cascade and events", () => {
  it("cascades to layers and records events", () => {
    const { db, dir } = tempDb();
    try {
      const r1 = db.transact((tx) => {
        const job = tx.assert(alice, worksAt, acme);
        tx.assert(job, confidence, 0.8);
      });
      const jobEid = (r1.results[0] as { eid: number }).eid;

      const r2 = db.transact((tx) => { tx.retract(jobEid); });

      // Both explicit retract and cascade of the confidence layer
      expect(r2.retracted.length).toBe(2);
      const kinds = r2.retracted.map((x) => x.kind).sort();
      expect(kinds).toContain("explicit");
      expect(kinds).toContain("cascade");

      // Events since tx 1
      const events = db.now().events(1);
      expect(events.length).toBe(2);
      expect(events.every((e) => e.op === "retract")).toBe(true);

      // Nothing left
      expect(db.now().triples()).toHaveLength(0);
    } finally { cleanup(dir); }
  });
});

// ---- (d) Speculate keeps nothing ----------------------------------------

describe("speculate", () => {
  it("answers queries hypothetically but commits nothing", () => {
    const { db, dir } = tempDb();
    try {
      const r = db.speculate(
        (tx) => { tx.assert(alice, worksAt, acme); },
        [
          { op: "triples" },
          {
            op: "sparql",
            text: "ASK { <urn:tiramemsu:v:alice> <urn:tiramemsu:v:worksAt> <urn:tiramemsu:v:acme> }",
          },
        ],
      );

      // Speculate sees the op's results
      expect((r.results[0] as unknown[]).length).toBe(1);
      const askR = r.results[1] as { kind: string; value: boolean };
      expect(askR.value).toBe(true);

      // Nothing was committed
      expect(db.now().triples()).toHaveLength(0);
      expect(db.now().events()).toHaveLength(0);
    } finally { cleanup(dir); }
  });
});

// ---- (e) dryRun ---------------------------------------------------------

describe("dryRun", () => {
  it("reports without committing", () => {
    const { db, dir } = tempDb();
    try {
      const r = db.transact(
        (tx) => { tx.assert(alice, worksAt, acme); },
        { dryRun: true },
      );

      expect(r.asserted.length).toBe(1);
      expect(db.now().triples()).toHaveLength(0);
    } finally { cleanup(dir); }
  });
});

// ---- (f) Errors carry .code ---------------------------------------------

describe("errors", () => {
  it("TiramemsuError carries a machine-readable code", () => {
    const { db, dir } = tempDb();
    try {
      // Parse error from invalid SPARQL
      expect(() => db.now().sparql("SELECT ?")).toThrow(TiramemsuError);
      let caught: TiramemsuError | undefined;
      try { db.now().sparql("SELECT ?"); } catch (e) { caught = e as TiramemsuError; }
      expect(caught?.code).toBe("Parse");
      expect(caught?.message.length).toBeGreaterThan(0);

      // NotLive: confirming a statement that does not exist
      let caught3: TiramemsuError | undefined;
      try { db.transact((tx) => { tx.confirm(99); }); } catch (e) { caught3 = e as TiramemsuError; }
      expect(caught3).toBeInstanceOf(TiramemsuError);
      expect(caught3?.code).toBe("NotLive");
    } finally { cleanup(dir); }
  });
});

// ---- (g) Big integer round trip ----------------------------------------

describe("big integer", () => {
  it("round-trips an integer beyond 2^53", () => {
    const { db, dir } = tempDb();
    try {
      const big = BigInt("9007199254740993"); // 2^53 + 1
      db.transact((tx) => { tx.assert(v("x"), v("big"), big); });

      const rows = db.now().triples({ s: v("x") });
      expect(rows.length).toBe(1);
      expect(rows[0].o).toBe(big);
      expect(typeof rows[0].o).toBe("bigint");
    } finally { cleanup(dir); }
  });
});

// ---- (h) Paths ----------------------------------------------------------

describe("paths", () => {
  it("reaches through a two-hop chain", () => {
    const { db, dir } = tempDb();
    try {
      db.transact((tx) => {
        tx.assert(v("a"), knows, v("b"));
        tx.assert(v("b"), knows, v("c"));
      });

      // v:knows uses the v: CURIE prefix (= urn:tiramemsu:v:)
      const rows = db.now().path(v("a"), "v:knows+");
      expect(rows.length).toBe(2); // reaches v:b and v:c

      // anyShortest mode returns hop details
      const shortest = db.now().path(v("a"), "v:knows+", { mode: "anyShortest" });
      expect(shortest.length).toBeGreaterThan(0);
      expect(shortest[0].path).not.toBeNull();
    } finally { cleanup(dir); }
  });
});

// ---- (i) Named graphs ---------------------------------------------------

describe("named graphs", () => {
  it("tags statements and queries graph membership", () => {
    const { db, dir } = tempDb();
    try {
      const session = v("session12");

      db.transact((tx) => {
        const job = tx.assert(alice, worksAt, acme);
        tx.addToGraph(job, session);
      });

      const graphs = db.now().graphs();
      expect(graphs.some((g) => JSON.stringify(g) === JSON.stringify(session))).toBe(true);

      const members = db.now().graphMembers(session);
      expect(members.length).toBe(1);
    } finally { cleanup(dir); }
  });
});

// ---- (j) Persistence across reopening ----------------------------------

describe("persistence", () => {
  it("data survives opening a second connection to the same file", () => {
    const dir = mkdtempSync(join(tmpdir(), "tiramemsu-persist-"));
    const path = join(dir, "persist.db");
    try {
      // Write with db1
      const db1 = Database.open(path);
      db1.transact((tx) => { tx.assert(alice, worksAt, acme); });

      // Read with db2 (separate connection to the same file)
      const db2 = Database.open(path);
      const rows = db2.now().triples();
      expect(rows.length).toBe(1);
      expect(JSON.stringify(rows[0].s)).toBe(JSON.stringify(alice));
      expect(JSON.stringify(rows[0].p)).toBe(JSON.stringify(worksAt));
      expect(JSON.stringify(rows[0].o)).toBe(JSON.stringify(acme));
    } finally { cleanup(dir); }
  });
});

// ---- Spec scenarios the first pass left out ------------------------------

describe("annotations, dates and immutable views", () => {
  it("reads a layer with SPARQL {| |} annotation syntax", () => {
    const { db, dir } = tempDb();
    try {
      db.transact((tx) => {
        const job = tx.assert(alice, worksAt, acme);
        tx.assert(job, confidence, 0.8);
      });
      const r = db.now().sparql(
        "SELECT ?c WHERE { <urn:tiramemsu:v:alice> <urn:tiramemsu:v:worksAt> <urn:tiramemsu:v:acme> " +
          "{| <urn:tiramemsu:v:confidence> ?c |} }",
      );
      if (r.kind !== "select") throw new Error("expected select");
      expect(r.rows.map((row) => row["c"])).toEqual([0.8]);
    } finally { cleanup(dir); }
  });

  it("round-trips a Date as an xsd:dateTime", () => {
    const { db, dir } = tempDb();
    try {
      const when = new Date("2026-09-30T12:34:56.000Z");
      db.transact((tx) => { tx.assert(v("x"), v("seenAt"), when); });
      const rows = db.now().triples({ s: v("x") });
      expect(rows.length).toBe(1);
      expect(rows[0].o).toBeInstanceOf(Date);
      expect((rows[0].o as Date).getTime()).toBe(when.getTime());
    } finally { cleanup(dir); }
  });

  it("validAt returns a new view and leaves the original unfiltered", () => {
    const { db, dir } = tempDb();
    try {
      db.transact((tx) => {
        tx.assert(alice, worksAt, acme, { validFrom: "2020-01-01", validTo: "2021-01-01" });
      });
      const plain = db.now();
      const filtered = plain.validAt("2026-01-01");
      expect(plain.triples({ s: alice }).length).toBe(1);
      expect(filtered.triples({ s: alice }).length).toBe(0);
      // the original still sees the statement after the filtered view was made
      expect(plain.triples({ s: alice }).length).toBe(1);
    } finally { cleanup(dir); }
  });
});

// ---- Features added after 0.1.0 -------------------------------------------

describe("paths: graphs and time-respecting", () => {
  it("stays inside the listed graphs", () => {
    const { db, dir } = tempDb();
    try {
      db.transact((tx) => {
        const ab = tx.assert(v("a"), knows, v("b"));
        tx.assert(v("b"), knows, v("c"));
        tx.addToGraph(ab, v("session12"));
      });
      const ends = (graphs?: ReturnType<typeof v>[]) =>
        db.now().path(v("a"), "v:knows+", { graphs }).map((r) => JSON.stringify(r.end));
      expect(ends([v("session12")])).toEqual([JSON.stringify(v("b"))]);
      expect(ends()).toHaveLength(2);
      expect(ends([v("nowhere")])).toEqual([]);
    } finally { cleanup(dir); }
  });

  it("reports the earliest arrival", () => {
    const { db, dir } = tempDb();
    try {
      const met = v("met");
      db.transact((tx) => {
        tx.assert(v("a"), met, v("b"), { validFrom: 1, validTo: 5 });
        tx.assert(v("b"), met, v("c"), { validFrom: 3, validTo: 9 });
      });
      const arrivals = (timeRespecting?: boolean | { after?: number }) =>
        db.now().path(v("a"), "v:met+", { timeRespecting }).map((r) => r.arrival);
      expect(arrivals(true)).toEqual([1, 3]);
      expect(arrivals({ after: 2 })).toEqual([2, 3]);
      expect(arrivals({ after: 6 })).toEqual([]);
      expect(arrivals()).toEqual([null, null]);
    } finally { cleanup(dir); }
  });

  it("runs temporal path syntax and reports completeness", () => {
    const dir = mkdtempSync(join(tmpdir(), "tiramemsu-test-"));
    const db = Database.open(join(dir, "test.db"), { pathMaxHops: 2 });
    try {
      const met = v("met");
      db.transact((tx) => {
        tx.assert(v("a"), met, v("b"), { validFrom: 1, validTo: 5 });
        tx.assert(v("b"), met, v("c"), { validFrom: 3, validTo: 9 });
        tx.assert(v("c"), met, v("d"), { validFrom: 4, validTo: 9 });
      });
      const api = db.now().path(v("a"), "v:met+", { timeRespecting: { after: 2 } }).map((r) => r.arrival);
      expect(api).toEqual([2, 3, 4]);
      // SPARQL with a start parameter
      const s = db.now().sparql(
        "SELECT ?t WHERE { SERVICE <urn:tiramemsu:tm:timeRespecting/$start> " +
          "{ v:a v:met+ ?y . ?y tm:arrival ?t } } ORDER BY ?t",
        { params: { start: 2 }, pathCompleteness: true },
      );
      if (s.kind !== "select") throw new Error("expected select");
      expect(s.rows.map((r) => r.t)).toEqual([2, 3, 4]);
      expect(s.pathCompleteness).toEqual({ kind: "exhaustive", maxHops: null, complete: true });
      // Cypher with a start parameter, capped at pathMaxHops = 2
      const c = db.now().cypher(
        "MATCH TIME RESPECTING AFTER $start ARRIVAL AS t (x {`@id`: 'v:a'})-[:met*]->(y) RETURN t ORDER BY t",
        { start: 2 },
        { pathCompleteness: true },
      );
      expect(c.rows).toEqual([[2], [3]]);
      expect(c.pathCompleteness).toEqual({ kind: "cap", maxHops: 2, complete: false });
      expect(db.now().cypher("MATCH (x) RETURN count(x) AS n").pathCompleteness).toBeUndefined();
      // the path report
      const capped = db.now().pathReport(v("a"), "v:met+", { mode: "trail", capped: true });
      expect(capped.rows).toHaveLength(2);
      expect(capped.completeness).toEqual({ kind: "cap", maxHops: 2, complete: false });
      const bound = db.now().pathReport(v("a"), "v:met+", { maxHops: 1 });
      expect(bound.completeness).toEqual({ kind: "bound", maxHops: 1, complete: true });
      expect(db.now().pathReport(v("a"), "v:met+").completeness.kind).toBe("exhaustive");
    } finally { cleanup(dir); }
  });
});

describe("provenance, dependents and bundles", () => {
  it("lists the statements behind each SPARQL row", () => {
    const { db, dir } = tempDb();
    try {
      db.transact((tx) => { tx.assert(v("a"), v("p"), v("b")); });
      const r = db.now().sparql("SELECT ?o WHERE { v:a v:p ?o }", { provenance: true });
      if (r.kind !== "select") throw new Error("expected select");
      expect(r.provenance).toEqual([[stmt(1)]]);
      expect(r.provenanceGaps).toEqual([]);
      const plain = db.now().sparql("SELECT ?o WHERE { v:a v:p ?o }");
      expect(plain.kind === "select" && plain.provenance).toBeFalsy();
      const path = db.now().sparql("SELECT ?o WHERE { v:a v:p+ ?o }", { provenance: true });
      expect(path.kind === "select" && path.provenanceGaps).toEqual(["recursivePath"]);
      try {
        db.now().sparql("INSERT DATA { v:x v:y v:z }", { queryOnly: true });
        throw new Error("expected a refusal");
      } catch (e) {
        expect(e).toBeInstanceOf(TiramemsuError);
        expect((e as TiramemsuError).code).toBe("Unsupported");
      }
    } finally { cleanup(dir); }
  });

  it("previews what a retraction would cascade to", () => {
    const { db, dir } = tempDb();
    try {
      const r = db.transact((tx) => {
        const job = tx.assert(alice, worksAt, acme);
        tx.assert(job, v("source"), "chat-1");
      });
      const job = r.asserted[0];
      expect(db.now().dependents(job)).toEqual(r.asserted);
      db.transact((tx) => { tx.retract(job); });
      expect(db.now().dependents(job)).toEqual([]);
      expect(db.asOf({ tx: 1 }).dependents(stmt(job))).toEqual(r.asserted);
    } finally { cleanup(dir); }
  });

  it("moves a belief with its layers and graphs to another database", () => {
    const a = tempDb();
    const b = tempDb();
    try {
      const r = a.db.transact((tx) => {
        const job = tx.assert(alice, worksAt, acme, { validFrom: "2020-01-01" });
        tx.assert(job, confidence, 0.8);
        tx.addToGraph(job, v("session12"));
      });
      const bundle = a.db.now().bundle(r.asserted[0]);
      expect(bundle.format).toBe("tiramemsu-bundle/1");
      const t = b.db.transact((tx) => {
        const fact = tx.importBundle(bundle);
        tx.assert(fact, v("importedFrom"), v("agentA"));
      });
      const imported = t.results[0] as { root: number; statements: Array<{ new: boolean }> };
      expect(imported.statements).toHaveLength(3);
      expect(imported.statements.every((s) => s.new)).toBe(true);
      expect(b.db.now().graphMembers(v("session12"))).toEqual([imported.root]);
    } finally { cleanup(a.dir); cleanup(b.dir); }
  });

  it("uses a statement as a graph name", () => {
    const { db, dir } = tempDb();
    try {
      const r = db.transact((tx) => {
        const edge = tx.assert(alice, knows, v("bob"));
        const fact = tx.assert(v("bob"), worksAt, acme);
        tx.addToGraph(fact, edge);
      });
      const [edge, fact] = r.asserted;
      expect(db.now().graphMembers(stmt(edge))).toEqual([fact]);
    } finally { cleanup(dir); }
  });
});

// ---- query budgets -------------------------------------------------------

describe("query budgets", () => {
  const code = (f: () => unknown): string | undefined => {
    try { f(); } catch (e) { return (e as TiramemsuError).code; }
    return undefined;
  };

  it("bounds reads and writes with typed error codes", () => {
    const { db, dir } = tempDb();
    try {
      db.transact((tx) => {
        for (let i = 0; i < 1500; i++) tx.assert(v(`n${i}`), v("p"), i);
      });
      const cross = "SELECT (COUNT(*) AS ?c) WHERE { ?a v:p ?x . ?b v:p ?y . ?c2 v:p ?z }";
      expect(code(() => db.now().withBudget({ timeoutMs: 100 }).sparql(cross))).toBe("DeadlineExceeded");
      const capped = db.now().withBudget({ maxRows: 10 });
      expect(code(() => capped.triples())).toBe("ResultLimitExceeded");
      expect(code(() => capped.sparql("SELECT ?s WHERE { ?s v:p ?o }"))).toBe("ResultLimitExceeded");
      // a fitting budget changes nothing, and the view keeps its time selection
      expect(capped.triples({ s: v("n1") })).toHaveLength(1);
      expect(db.asOf({ tx: 0 }).withBudget({ maxRows: 1 }).triples()).toEqual([]);
      // a stopped write commits nothing
      const before = db.now().triples({ p: v("q") }).length;
      expect(code(() => db.cypherWrite(
        "MATCH (a), (b), (c) WHERE a.p >= 0 AND b.p >= 0 AND c.p >= 0 CREATE (a)-[:q]->(b)",
        undefined,
        { budget: { timeoutMs: 100 } },
      ))).toBe("DeadlineExceeded");
      expect(db.cancel("pre")).toBe(false);
      expect(code(() => db.transact((tx) => { tx.assert(v("x"), v("q"), v("y")); },
        { budget: { cancelKey: "pre" } }))).toBe("Cancelled");
      expect(db.now().triples({ p: v("q") })).toHaveLength(before);
      // budget and transaction options travel together
      const r = db.transact((tx) => { tx.assert(v("x"), v("q"), v("y")); },
        { dryRun: true, budget: { timeoutMs: 10000 } });
      expect(r.asserted).toHaveLength(1);
      expect(db.now().triples({ p: v("q") })).toHaveLength(before);
    } finally { cleanup(dir); }
  });

  it("opens with a reader timeout", () => {
    const dir = mkdtempSync(join(tmpdir(), "tiramemsu-test-"));
    try {
      const db = Database.open(join(dir, "r.db"), { readers: 1, readerTimeoutMs: 50 });
      expect(db.now().triples()).toEqual([]);
      expect(db.info().readers).toBe(1);
    } finally { cleanup(dir); }
  });
});

describe("bulk import", () => {
  const code = (f: () => unknown): string | undefined => {
    try { f(); return undefined; } catch (e) { return (e as TiramemsuError).code; }
  };

  it("commits chunks, refuses other writes and analyses once on finish", () => {
    const { db, dir } = tempDb();
    try {
      const imp = db.bulkImport();
      try {
        const r = imp.chunk((tx) => { for (let i = 0; i < 5; i++) tx.assert(v(`a${i}`), v("p"), i); });
        expect(r.t).toBe(1);
        expect(r.progress.chunks).toBe(1);
        imp.chunk([{ op: "assert", s: v("b"), p: v("p"), o: 1 }]);
        expect(code(() => imp.chunk((tx) => { tx.confirm(99); }))).toBe("NotLive");
        expect(code(() => db.transact((tx) => { tx.assert(v("x"), v("p"), 1); }))).toBe("ImportInProgress");
        expect(code(() => db.bulkImport())).toBe("ImportInProgress");
        expect(db.info().importActive).toBe(true);
        expect(db.now().triples()).toHaveLength(6);
        const p = imp.progress();
        expect([p.chunks, p.rejected, p.asserted]).toEqual([2, 1, 6]);
      } finally {
        const s = imp.finish();
        expect(s.analyzed).toBe(true);
        expect(s.maintenanceError).toBeNull();
        expect(s.progress.txs).toEqual([1, 2]);
      }
      expect(db.info().importActive).toBe(false);
      db.transact((tx) => { tx.assert(v("x"), v("p"), 1); });

      const second = db.bulkImport();
      second.chunk((tx) => { tx.assert(v("c"), v("p"), 1); });
      expect(second.cancel().chunks).toBe(1);
      expect(db.info().statisticsDue).toBe(true);
      expect(db.now().triples()).toHaveLength(8);
    } finally { cleanup(dir); }
  });
});

// ---- text recall ------------------------------------------------------------

describe("text recall", () => {
  it("recalls ranked statements with evidence from the API, SPARQL and Cypher", () => {
    const dir = mkdtempSync(join(tmpdir(), "tiramemsu-test-"));
    try {
      const db = Database.open(join(dir, "t.db"), { textIndex: true });
      const r = db.transact((tx) => {
        const n = tx.assert(alice, v("note"), "met at the lisbon offsite");
        tx.assert(n, confidence, 0.9);
        tx.assert(v("bob"), v("note"), "lisbon");
      });
      const hits = db.now().textSearch("lisbon");
      expect(hits).toHaveLength(2);
      const a = hits.find((h) => h.eid === r.asserted[0])!;
      expect(a.text).toBe("met at the lisbon offsite");
      expect(a.s).toEqual(alice);
      expect(a.evidence.confidence).toBe(0.9);
      const b = hits.find((h) => h.eid === r.asserted[2])!;
      expect(b.evidence.confidence).toBeNull();
      expect(hits.map((h) => h.rank)).toEqual([1, 2]);
      expect(db.now().textSearch("offsite nowhere", { mode: "any", limit: 1 })).toHaveLength(1);
      expect(db.now().textSearch("lisbon", { graphs: [v("nowhere")] })).toEqual([]);
      const s = db.now().sparql('SELECT ?e WHERE { ?e tm:textMatch "lisbon" ; tm:textRank ?r } ORDER BY ?r');
      expect((s as { rows: unknown[] }).rows).toHaveLength(2);
      const c = db.now().cypher("CALL tiramemsu.text.search('lisbon') YIELD rank RETURN rank");
      expect(c.rows).toEqual([[1], [2]]);
      expect(db.rebuildTextIndex()).toBe(2);
      expect(db.enableTextIndex()).toBe(false);
      expect(db.now().textSearch("lisbon").map((h) => h.eid)).toEqual(hits.map((h) => h.eid));
    } finally { cleanup(dir); }
  });

  it("reports a missing index with a typed code", () => {
    const { db, dir } = tempDb();
    try {
      db.transact((tx) => { tx.assert(alice, v("note"), "not indexed yet"); });
      let code: string | undefined;
      try { db.now().textSearch("indexed"); } catch (e) { code = (e as TiramemsuError).code; }
      expect(code).toBe("TextIndexUnavailable");
      expect(db.enableTextIndex()).toBe(true);
      expect(db.now().textSearch("indexed")).toHaveLength(1);
    } finally { cleanup(dir); }
  });
});

// ---- saved answers ---------------------------------------------------------

describe("saved answers", () => {
  it("marks a saved answer stale when its support is retracted, and refreshes it", () => {
    const { db, dir } = tempDb();
    try {
      const r = db.transact((tx) => { tx.assert(alice, worksAt, acme); });
      const a = db.saveAnswer("employer", { text: "SELECT ?o WHERE { v:alice v:worksAt ?o }" });
      expect(a.status).toBe("fresh");
      expect(a.dependencies).toEqual([r.asserted[0]]);
      expect(a.coverage).toEqual(["mutableView"]);
      expect(a.result).toMatchObject({ kind: "select", rows: [{ o: acme }] });
      // a parameterized Cypher answer on a fixed view keeps both
      const c = db.saveAnswer("count", {
        language: "cypher",
        text: "MATCH (p)-[:worksAt]->(c) WHERE $min >= 0 RETURN count(*) AS n",
        params: { min: 1 },
        view: db.asOf({ tx: 1 }),
      });
      expect(c.params).toEqual({ min: 1 });
      expect(c.view).toEqual({ kind: "asOf", tx: 1 });
      // a new matching row: recheck; then the retraction: stale
      db.transact((tx) => { tx.assert(alice, worksAt, globex); });
      const first = db.checkSavedAnswers();
      expect(first.map((m) => [m.name, m.status, m.cause])).toEqual([["employer", "recheck", "insertion"]]);
      db.transact((tx) => { tx.retract(r.asserted[0]); });
      const second = db.checkSavedAnswers();
      expect(second).toHaveLength(1);
      expect(second[0].status).toBe("stale");
      expect(second[0].cause).toBe("supportRetracted");
      expect(second[0].event).toMatchObject({ eid: r.asserted[0], op: "retract", kind: "explicit" });
      expect(db.checkSavedAnswers()).toEqual([]);
      // a failed refresh keeps the mark
      db.cancel("refresh-1");
      let code: string | undefined;
      try { db.refreshAnswer("employer", { cancelKey: "refresh-1" }); } catch (e) { code = (e as TiramemsuError).code; }
      expect(code).toBe("Cancelled");
      expect(db.savedAnswer("employer")!.status).toBe("stale");
      expect(db.savedAnswer("employer")!.checkpoint).toBe(1);
      const fresh = db.refreshAnswer("employer");
      expect(fresh.status).toBe("fresh");
      expect(fresh.revision).toBe(2);
      expect(fresh.result).toMatchObject({ rows: [{ o: globex }] });
      expect(db.savedAnswers().map((s) => s.name)).toEqual(["count", "employer"]);
      expect(db.deleteSavedAnswer("count")).toBe(true);
      expect(db.savedAnswer("count")).toBeNull();
      try { db.refreshAnswer("count"); } catch (e) { code = (e as TiramemsuError).code; }
      expect(code).toBe("SavedAnswerNotFound");
    } finally { cleanup(dir); }
  });
});

// ---- native cyclic joins --------------------------------------------------------

describe("native cyclic joins", () => {
  // @lat: [[tests#Cyclic Joins#Bindings Expose LFTJ Routing]]
  it("opts in with lftj, explains the route and returns the SQL rows", () => {
    const dir = mkdtempSync(join(tmpdir(), "tiramemsu-test-"));
    try {
      const path = join(dir, "t.db");
      const tri = "SELECT ?x ?y ?z WHERE { ?x v:k ?y . ?y v:k ?z . ?z v:k ?x }";
      const plain = Database.open(path);
      plain.now().sparql("INSERT DATA { v:a v:k v:b . v:b v:k v:c . v:c v:k v:a . v:a v:k v:c }");
      const base = plain.now().explainSparql(tri);
      expect(base.regions.some((r) => r.note === "cyclicLftjDisabled")).toBe(true);
      const rows = (db: Database) =>
        ((db.now().sparql(tri) as { rows: unknown[] }).rows.map((r) => JSON.stringify(r))).sort();
      const want = rows(plain);
      expect(want).toHaveLength(3);
      const native = Database.open(path, { lftj: true, lftjMinRows: 0 });
      const ex = native.now().explainSparql(tri);
      const region = ex.regions.find((r) => r.kind === "nativeLftj");
      expect(region?.note).toBe("lftjNative");
      expect(ex.sql).toContain("tm_lftj(");
      expect(rows(native)).toEqual(want);
    } finally { cleanup(dir); }
  });
});
