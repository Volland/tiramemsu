// @tiramemsu/node — integration tests covering all required scenarios.
// Run after `npm run build:native` (the .node file must exist).

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it, expect } from "vitest";
import { Database, iri, TiramemsuError } from "../lib/index.ts";

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
