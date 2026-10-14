// Load a real agent execution trace into Tiramemsu as a causal-temporal
// event graph (CTEG), then ask it questions in SPARQL.
//
//   node cteg-demo.mjs [trace.json] [out.db]
//
// Uses the Node binding from this repository. In your own project:
//   import { Database, iri } from "@tiramemsu/node";
import { createRequire } from "node:module";
import { readFileSync, rmSync } from "node:fs";

const require = createRequire(import.meta.url);
const { Database, iri } = require("../../../bindings/node/dist/index.js");

const tracePath = process.argv[2] ?? new URL("trace.json", import.meta.url).pathname;
const dbPath = process.argv[3] ?? "cteg.db";
for (const f of [dbPath, `${dbPath}-wal`, `${dbPath}-shm`]) rmSync(f, { force: true });

const v = (name) => iri(`urn:tiramemsu:v:${name}`);
const A = iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type");
const ev = (id) => v(`ev-${id.slice(0, 8)}`);

const trace = JSON.parse(readFileSync(tracePath, "utf8"));
const db = Database.open(dbPath);

// ---- the recorder: one emission = one transaction --------------------------
//
// An event is a node. Its type is a statement whose valid time is when the
// event happened (a point, or a span for calls that run for a while). Its
// single causal parent is a `v:caused` statement whose valid time is the
// child's instant. No timestamp column: time lives on the statements.
function record(e, { spanTo, latencyMs } = {}) {
  return db.transact((tx) => {
    tx.meta(v("recordedBy"), "cteg-demo replay");
    tx.assert(ev(e.id), A, v(e.type), { validFrom: e.t, validTo: spanTo ?? e.t + 1 });
    tx.assert(ev(e.id), v("inSession"), v(`session-${e.session}`));
    tx.assert(ev(e.id), v("agent"), e.agent);
    if (e.tool) tx.assert(ev(e.id), v("tool"), e.tool);
    if (e.label) tx.assert(ev(e.id), v("label"), e.label);
    // What the harness logged: the raw parentUuid chain, kept as evidence.
    if (e.parent) tx.assert(ev(e.parent), v("loggedBefore"), ev(e.id), { validFrom: e.t, validTo: e.t + 1 });
    // The CTEG edge, under our local parenthood rule (see derive below).
    if (e.cause) {
      const edge = tx.assert(ev(e.cause), v("caused"), ev(e.id), { validFrom: e.t, validTo: e.t + 1 });
      if (latencyMs !== undefined) tx.assert(edge, v("latencyMs"), latencyMs);
    }
    for (const input of e.consumed ?? []) tx.assert(ev(e.id), v("consumed"), ev(input));
  });
}

// The same recorder in SPARQL Update. Valid time is an annotation on the
// statement it bounds, so one INSERT DATA writes the event and its edge.
const lit = (x) => (typeof x === "number" ? String(x) : JSON.stringify(x));
function recordSparql(e, { spanTo, latencyMs } = {}) {
  const ev_ = `v:ev-${e.id.slice(0, 8)}`;
  const when = (from, to) => `{| tm:validFrom ${from} ; tm:validTo ${to} |}`;
  const lines = [
    `${ev_} a v:${e.type} ${when(e.t, spanTo ?? e.t + 1)} .`,
    `${ev_} v:inSession v:session-${e.session} ; v:agent ${lit(e.agent)} .`,
  ];
  if (e.tool) lines.push(`${ev_} v:tool ${lit(e.tool)} .`);
  if (e.label) lines.push(`${ev_} v:label ${lit(e.label)} .`);
  if (e.parent) lines.push(`v:ev-${e.parent.slice(0, 8)} v:loggedBefore ${ev_} ${when(e.t, e.t + 1)} .`);
  if (e.cause) {
    const extra = latencyMs !== undefined ? ` ; v:latencyMs ${latencyMs}` : "";
    lines.push(`v:ev-${e.cause.slice(0, 8)} v:caused ${ev_} {| tm:validFrom ${e.t} ; tm:validTo ${e.t + 1}${extra} |} .`);
  }
  for (const input of e.consumed ?? []) lines.push(`${ev_} v:consumed v:ev-${input.slice(0, 8)} .`);
  return db.now().sparql(`INSERT DATA {\n${lines.join("\n")}\n}`).report;
}

function graftSparql(callId, subRoot, agentName) {
  return db.now().sparql(`INSERT DATA {
  v:ev-${callId.slice(0, 8)} v:caused v:ev-${subRoot.id.slice(0, 8)}
    {| tm:validFrom ${subRoot.t} ; tm:validTo ${subRoot.t + 1} ; v:grafts ${lit(agentName)} |} .
}`).report;
}

const SPARQL = process.env.RECORDER === "sparql";

// Grafting a finished subagent trace: one edge from the invoking event to the
// subagent's root, written when the result reaches the parent.
function graft(callId, subRoot, agentName) {
  return db.transact((tx) => {
    tx.meta(v("recordedBy"), "cteg-demo replay");
    tx.meta(v("reason"), `graft ${agentName} trace on return`);
    const edge = tx.assert(ev(callId), v("caused"), ev(subRoot.id), { validFrom: subRoot.t, validTo: subRoot.t + 1 });
    tx.assert(edge, v("grafts"), agentName);
  });
}

// ---- derive spans, latencies and the multi-parent overlay ------------------
const all = [...trace.main, ...trace.subagents.flatMap((s) => s.events)];
const byToolUse = new Map(all.filter((e) => e.type === "ToolResult").map((e) => [e.toolUseId, e]));
const subCall = trace.main.find((e) => e.type === "SubagentCall");
const subReturn = trace.main.find((e) => e.type === "SubagentReturn");

// Local parenthood rule: harness context (hook output, reminders) is logged
// into the chain but causes nothing. An event's cause is its nearest logged
// ancestor that is not harness context; context events get no cause.
const byId = new Map(all.map((e) => [e.id, e]));
for (const e of all) {
  if (e.type === "HarnessContext") continue;
  let p = e.parent ? byId.get(e.parent) : undefined;
  while (p && p.type === "HarnessContext") p = p.parent ? byId.get(p.parent) : undefined;
  e.cause = p?.id;
}
// Second rule, the paper's: what comes back from a subagent continues from the
// invocation node, not from whatever the parent was doing when it arrived.
subReturn.cause = subCall.id;

// A model step consumes every tool result that arrived since the previous step
// of the same agent. Under single parenthood only one of them can be its parent.
const MODEL = new Set(["Thinking", "Message", "ToolCall", "SubagentCall"]);
for (const events of [trace.main, ...trace.subagents.map((s) => s.events)]) {
  let pending = [];
  for (const e of [...events].sort((a, b) => a.t - b.t)) {
    if (e.type === "ToolResult") pending.push(e.id);
    else if (MODEL.has(e.type) && pending.length) { e.consumed = pending; pending = []; }
  }
}

// ---- replay in time order ---------------------------------------------------
const steps = all.map((e) => ({ at: e.t, e }));
for (const s of trace.subagents) {
  const root = s.events.find((e) => e.type !== "HarnessContext" && !e.cause);
  steps.push({ at: subReturn.t, graft: { callId: subCall.id, root, name: `fact-checker ${s.agent}` } });
}
steps.sort((a, b) => a.at - b.at);

let graftTx = null;
for (const s of steps) {
  if (s.graft) {
    graftTx = (SPARQL ? graftSparql : graft)(s.graft.callId, s.graft.root, s.graft.name).t;
    continue;
  }
  const e = s.e;
  let opts = {};
  if (e.type === "ToolCall") {
    const r = byToolUse.get(e.toolUseId);
    if (r) opts.spanTo = Math.max(r.t, e.t + 1);
  }
  if (e.type === "SubagentCall") opts.spanTo = subReturn.t;
  if (e.type === "SubagentReturn") opts.latencyMs = e.t - subCall.t;
  if (e.type === "ToolResult") {
    const call = all.find((c) => c.toolUseId === e.toolUseId && c.type !== "ToolResult");
    if (call && call.id === e.cause) opts.latencyMs = e.t - call.t;
  }
  (SPARQL ? recordSparql : record)(e, opts);
}

// ---- queries ----------------------------------------------------------------
const rootId = trace.main.find((e) => e.type !== "HarnessContext" && !e.cause).id;
const ROOT = `v:ev-${rootId.slice(0, 8)}`;
const short = (x) =>
  x === undefined ? "" :
  typeof x === "object" && x.iri ? x.iri.replace("urn:tiramemsu:v:", "") :
  typeof x === "object" && x.stmt !== undefined ? `stmt:${x.stmt}` :
  x instanceof Date ? x.toISOString().slice(11, 23) :
  typeof x === "object" && Object.keys(x).length === 1 ? Object.values(x)[0] :
  typeof x === "bigint" ? Number(x) : x;
function run(title, view, q, opts) {
  const r = view.sparql(q, opts);
  console.log(`\n### ${title}\n${q.trim()}\n--`);
  if (r.kind === "ask") { console.log(r.value); return r; }
  console.log(r.vars.join(" | "));
  for (const row of r.rows.slice(0, 15)) console.log(r.vars.map((k) => short(row[k])).join(" | "));
  if (r.rows.length > 15) console.log(`… ${r.rows.length} rows`);
  else console.log(`(${r.rows.length} rows)`);
  return r;
}

const now = db.now();
console.log(`root ${ROOT}, graft committed in tx ${graftTx}, last tx ${short(now.sparql("SELECT (MAX(?t) AS ?m) WHERE { ?e tm:txAdded ?t }").rows[0].m)}`);

run("1. Rebuild the session: every event, its parent and its time", now, `
SELECT ?t ?type ?label ?parent WHERE {
  ?e v:inSession v:session-main ; a ?type .
  OPTIONAL { ?e v:label ?label }
  OPTIONAL { ?parent v:caused ?e ~ ?edge . }
  ?e a ?type ~ ?occ . ?occ tm:validFrom ?t .
  FILTER(?type != v:HarnessContext)
} ORDER BY ?t`);

run("2. How big is it: events per type (whole graph, both agents)", now, `
SELECT ?type (COUNT(?e) AS ?n) WHERE { ?e a ?type } GROUP BY ?type ORDER BY DESC(?n)`);

const lastSubCall = now.sparql(`
SELECT ?e WHERE { ?e a v:ToolCall ~ ?o ; v:inSession ?s . ?o tm:validFrom ?t FILTER(?s != v:session-main) }
ORDER BY DESC(?t) LIMIT 1`).rows[0].e.iri.replace("urn:tiramemsu:v:", "v:");

run("3. Why did this happen? Typed causal ancestors of the subagent's last tool call", now, `
SELECT ?anc ?type ?label WHERE {
  ?anc v:caused+ ${lastSubCall} ; a ?type .
  OPTIONAL { ?anc v:label ?label }
  FILTER(?type IN (v:Prompt, v:SubagentCall, v:ToolCall))
}`);

run("4. What did it cause? Everything downstream of the subagent call", now, `
SELECT ?agent (COUNT(?d) AS ?n) WHERE {
  ?call a v:SubagentCall . ?call v:caused+ ?d . ?d v:agent ?agent
} GROUP BY ?agent`);

for (const [name, pred] of [["raw log", "v:loggedBefore"], ["CTEG", "v:caused"]]) {
  run(`5a. ${name}: links that do not go strictly forward in time`, now, `
SELECT (COUNT(*) AS ?backwards) WHERE {
  ?p ${pred} ?c ~ ?link . ?link tm:validFrom ?tc .
  ?p a ?pt ~ ?occ . ?occ tm:validFrom ?tp .
  FILTER(?tc <= ?tp)
}`);
  const logRoot = name === "raw log" ? `v:ev-${trace.main.find((e) => e.parent === null).id.slice(0, 8)}` : ROOT;
  run(`5b. ${name}: reachable from the root, but not by a forward-in-time journey`, now, `
SELECT (COUNT(?e) AS ?reachable) (SUM(?cut) AS ?cutOff) WHERE {
  ${logRoot} ${pred}+ ?e .
  BIND(IF(EXISTS { SERVICE <urn:tiramemsu:tm:timeRespecting> { ${logRoot} ${pred}+ ?e } }, 0, 1) AS ?cut)
}`);
}

run("6. Earliest arrival: when could each tool call first have been influenced by the prompt?", now, `
SELECT ?tool ?label ?arrived WHERE {
  SERVICE <urn:tiramemsu:tm:timeRespecting> {
    ${ROOT} v:caused+ ?call .
    ?call tm:arrival ?arrived
  }
  ?call a v:ToolCall ; v:tool ?tool .
  FILTER(?tool IN ("WebFetch", "WebSearch"))
  OPTIONAL { ?call v:label ?label }
} ORDER BY ?arrived`);

run("7. Annotated edges: the slowest tool calls (latency lives on the causal link)", now, `
SELECT ?tool ?label ?ms WHERE {
  ?call v:caused ?result {| v:latencyMs ?ms |} .
  ?call v:tool ?tool .
  OPTIONAL { ?call v:label ?label }
} ORDER BY DESC(?ms) LIMIT 5`);

const midSub = Math.round((subCall.t + subReturn.t) / 2);
run(`8. What was in flight at ${new Date(midSub).toISOString()}?`, now, `
SELECT ?type ?agent ?label
FROM <urn:tiramemsu:tm:validAt/${new Date(midSub).toISOString()}>
WHERE { ?e a ?type ; v:agent ?agent . OPTIONAL { ?e v:label ?label }
        FILTER(?type IN (v:ToolCall, v:SubagentCall)) }`);

const before = db.asOf({ tx: graftTx - 1 });
run(`9a. Before the graft (as of tx ${graftTx - 1}): subagent events reachable from the prompt`, before, `
SELECT (COUNT(?e) AS ?n) WHERE { ${ROOT} v:caused+ ?e . ?e v:agent "fact-checker" }`);
run(`9b. Before the graft: subagent events already stored`, before, `
SELECT (COUNT(?e) AS ?n) WHERE { ?e v:agent "fact-checker" ; a ?t FILTER(?t != v:HarnessContext) }`);
run("9c. After the graft (now)", now, `
SELECT (COUNT(?e) AS ?n) WHERE { ${ROOT} v:caused+ ?e . ?e v:agent "fact-checker" }`);
run("9d. The graft edge itself: when it was true, when we learned it", now, `
SELECT ?call ?root ?validFrom ?addedAt ?tx WHERE {
  ?call v:caused ?root ~ ?g . ?g v:grafts ?name ;
     tm:validFrom ?validFrom ; tm:addedAt ?addedAt ; tm:txAdded ?tx .
}`);

run("10. Beyond single parenthood: inputs a step consumed that are not its causal ancestors", now, `
SELECT ?stepType ?inputTool (COUNT(*) AS ?n) WHERE {
  ?step v:consumed ?input ; a ?stepType .
  ?call v:caused ?input . ?call v:tool ?inputTool .
  MINUS { ?step v:consumed ?input . ?input v:caused+ ?step }
} GROUP BY ?stepType ?inputTool ORDER BY DESC(?n)`);

run(`11. Prefix property: the trace as of tx ${graftTx - 1}, mid-run, before the subagent returned`, before, `
SELECT (COUNT(?e) AS ?events) (SUM(?bad) AS ?backwards) WHERE {
  ?p v:caused ?e ~ ?link . ?link tm:validFrom ?te .
  ?p a ?pt ~ ?occ . ?occ tm:validFrom ?tp .
  BIND(IF(?te <= ?tp, 1, 0) AS ?bad)
}`);

run("12. Single parenthood holds: events with more than one cause", now, `
SELECT ?e (COUNT(?p) AS ?parents) WHERE { ?p v:caused ?e } GROUP BY ?e HAVING (COUNT(?p) > 1)`);

if (!SPARQL) {
  const cy = (title, q) => {
    const r = db.now().cypher(q);
    console.log(`\n### ${title}\n${q.trim()}\n--`);
    console.log(r.columns.join(" | "));
    for (const row of r.rows) console.log(row.map(short).join(" | "));
    console.log(`(${r.rows.length} rows)`);
  };
  cy("13. Cypher: the graft edge, its statement id and the transaction that wrote it", `
MATCH (call)-[g:caused]->(root)
WHERE g.grafts IS NOT NULL
WITH g, tiramemsu.transaction(g) AS tx
RETURN tiramemsu.statementId(g) AS id, g.validFrom AS validFrom,
       tx.number AS tx, tx.instant AS recordedAt, tx.reason AS why`);
  run("14. SPARQL: the same edge, the same transaction", now, `
SELECT ?g ?tx ?why WHERE {
  ?call v:caused ?root ~ ?g . ?g v:grafts ?name ; tm:txAdded ?tx .
  ?tx v:reason ?why
}`);
}
