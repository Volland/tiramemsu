// Smoke test of the generated JavaScript package, the way a worker uses it:
//   wasm-bindgen --target nodejs --out-dir <pkg> tiramemsu_wasm.wasm
//   node bindings/wasm/tests/node-smoke.cjs <pkg>
// (memory storage: Node.js has no OPFS)
const assert = require("node:assert/strict");
const path = require("node:path");

const pkg = require(path.resolve(process.argv[2], "tiramemsu_wasm.js"));

(async () => {
  const info = JSON.parse(pkg.runtimeInfo());
  assert.equal(info.capabilities.vtab, true);
  assert.equal(info.capabilities.readerPool, false);

  const config = JSON.stringify({ storage: "memory", path: "smoke.db", journal: "rollback" });
  const db = await pkg.Database.open(config);
  const iri = (s) => ({ iri: `urn:tiramemsu:v:${s}` });
  const report = JSON.parse(
    db.call("transact", JSON.stringify({ ops: [{ op: "assert", s: iri("alice"), p: iri("knows"), o: iri("bob") }] })),
  );
  assert.equal(report.t, 1);
  const res = JSON.parse(
    db.call("sparql", JSON.stringify({ view: { kind: "now" }, text: "SELECT ?o WHERE { v:alice v:knows ?o }" })),
  );
  assert.deepEqual(res.rows, [{ o: iri("bob") }]);

  // errors are thrown as tiramemsu:{code,message}
  assert.throws(() => db.call("importBegin", "{}"), (e) => {
    const err = JSON.parse(e.message.slice("tiramemsu:".length));
    return err.code === "Unsupported";
  });
  await assert.rejects(
    pkg.Database.open(JSON.stringify({ storage: "memory", path: "wal.db", journal: "wal" })),
    (e) => e.message.includes("MissingCapability"),
  );

  // the file round-trips through bytes
  const bytes = db.exportFile();
  assert.ok(bytes instanceof Uint8Array && bytes.length > 0);
  db.close();
  const copyConfig = JSON.stringify({ storage: "memory", path: "copy.db", journal: "rollback" });
  await pkg.importFile(copyConfig, bytes);
  const copy = await pkg.Database.open(copyConfig);
  const again = JSON.parse(copy.call("triples", JSON.stringify({ view: { kind: "now" } })));
  assert.equal(again.length, 1);
  assert.deepEqual(again[0].o, iri("bob"));
  assert.equal(again[0].tAdd, 1);
  console.log("tiramemsu-wasm node smoke: ok");
})().catch((e) => {
  console.error(e);
  process.exit(1);
});
