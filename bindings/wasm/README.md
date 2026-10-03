# tiramemsu-wasm

The WebAssembly binding of tiramemsu: the whole engine (transactions, temporal views, SPARQL, Cypher, paths, text recall, bundles, saved answers) running in a Web Worker on SQLite compiled to WebAssembly, with the database in memory or in the Origin Private File System.

It is the [JSON bridge](https://github.com/Volland/tiramemsu/tree/main/bindings/json) that the Node.js and Python bindings use, served on the [`tm-wasm`](https://github.com/Volland/tiramemsu/tree/main/crates/tm-wasm) host. Every call is synchronous, so the database lives in a worker and the page talks to it with messages. The files are the native format: export one from the browser and open it with the Rust, Node.js or Python binding, or import a native file.

## Build

The package is built with wasm-bindgen and is not published (neither to crates.io nor to npm).

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked   # the version in Cargo.lock
cargo build -p tiramemsu-wasm --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir pkg \
  target/wasm32-unknown-unknown/release/tiramemsu_wasm.wasm
```

`pkg/tiramemsu_wasm.js` exports the class `Database` and the functions `importFile` and `runtimeInfo`.

## API

| Call | Result |
|---|---|
| `await Database.open(configJson)` | an open database; OPFS is installed on first use |
| `db.call(op, argsJson)` | the JSON result text of one bridge operation |
| `db.capabilities()` | `{"storage","journal","queryEngine","readerPool","functions","vtab","stat4","fts5"}` |
| `db.exportFile()` | the committed file as a `Uint8Array` |
| `db.close()` | closes; later calls throw |
| `await importFile(configJson, bytes)` | stores a database file as the configured path, before `open` |
| `runtimeInfo()` | SQLite version, compile options and probed capabilities |

The configuration is `{"storage": "memory" | "opfs", "path": "memory.db", "journal": "rollback", "queryEngine": true, "opfs": {"directory", "capacity", "clearOnInit"}, "options": {...}}`. `journal` is required: both browser VFSes keep a rollback journal, and `"wal"` fails at open with `MissingCapability` instead of silently using another mode. `options` are the bridge's open options (`termCacheCapacity`, `pathMaxHops`, `textIndex`, `lftj`, ...) without `readers` and `readerTimeoutMs`, as there is no reader pool.

The operations and their arguments are those of the Node.js binding's `call` (`transact`, `sparql`, `cypher`, `triples`, `path`, `textSearch`, `bundle`, `conflicts`, `saveAnswer`, ...). Errors are thrown as `Error` objects whose message is `tiramemsu:{"code","message"}`. Three things are refused with `Unsupported`, because `wasm32-unknown-unknown` has no `std::time::Instant` and a worker runs one call at a time: a `budget` argument, the bulk import operations (`importBegin`, ...), and `cancel`.

## Web Worker example

`worker.js`, loaded with `new Worker("worker.js", { type: "module" })`:

```js
import init, { Database, importFile } from "./pkg/tiramemsu_wasm.js";

await init();
// OPFS (durable) needs a dedicated worker; use "memory" anywhere else
const config = JSON.stringify({ storage: "opfs", path: "/memory.db", journal: "rollback" });
const db = await Database.open(config);

self.onmessage = ({ data: { id, op, args } }) => {
  try {
    if (op === "export") {
      const bytes = db.exportFile();
      self.postMessage({ id, bytes }, [bytes.buffer]);
    } else {
      self.postMessage({ id, result: JSON.parse(db.call(op, JSON.stringify(args ?? {}))) });
    }
  } catch (e) {
    const m = String(e.message);
    self.postMessage({ id, error: m.startsWith("tiramemsu:") ? JSON.parse(m.slice(10)) : { code: "Error", message: m } });
  }
};
```

The page:

```js
const worker = new Worker("worker.js", { type: "module" });
let next = 0;
const pending = new Map();
worker.onmessage = ({ data }) => {
  const { resolve, reject } = pending.get(data.id);
  pending.delete(data.id);
  data.error ? reject(data.error) : resolve(data.result ?? data.bytes);
};
const call = (op, args) =>
  new Promise((resolve, reject) => {
    pending.set(++next, { resolve, reject });
    worker.postMessage({ id: next, op, args });
  });

const iri = (s) => ({ iri: `urn:tiramemsu:v:${s}` });
await call("transact", { ops: [{ op: "assert", s: iri("alice"), p: iri("knows"), o: iri("bob") }] });
const { rows } = await call("sparql", {
  view: { kind: "now" },
  text: "SELECT ?o WHERE { v:alice v:knows ?o }",
});
// download the database; it opens natively with tiramemsu
const bytes = await call("export");
```

## Tests

`cargo test -p tiramemsu-wasm` runs the binding's logic natively; `cargo test -p tiramemsu-wasm --target wasm32-unknown-unknown --tests` drives the JavaScript surface in WebAssembly under Node.js. The OPFS storage is tested in a dedicated worker in headless Chrome by `tm-wasm` (`--features opfs-tests`).

Part of [tiramemsu](https://github.com/Volland/tiramemsu). Licensed under MIT or Apache-2.0.
