# tiramemsu-mcp

**A local MCP server that gives an agent a tiramemsu memory file as typed tools.**

[Tiramemsu](https://github.com/Volland/tiramemsu) is a bitemporal, never-forget triple store on one SQLite file: every fact is a statement with its own id, layers (sources, confidence, beliefs) are statements about statements, and corrections keep the old version visible in time. `tiramemsu-mcp` serves one such file to an MCP client (Claude Code, Claude Desktop, or any client that speaks MCP over stdio) as fourteen tools. It is a separate, opt-in package: the database crates carry no protocol code.

## Install

```sh
cargo install tiramemsu-mcp
```

MSRV 1.88. SQLite is bundled, so the binary has no system dependencies.

## Register it

Claude Code:

```sh
claude mcp add tiramemsu -- tiramemsu-mcp --db ./memory.db
# read-only, with text search:
claude mcp add tiramemsu-ro -- tiramemsu-mcp --db ./memory.db --read-only --text-index
```

Claude Desktop (`claude_desktop_config.json`; use an absolute path, since the working directory is not yours):

```json
{
  "mcpServers": {
    "tiramemsu": {
      "command": "tiramemsu-mcp",
      "args": ["--db", "/Users/me/memory.db", "--text-index"]
    }
  }
}
```

## Options

| Flag | Meaning | Default |
|---|---|---|
| `--db <path>` | The database file, created if missing. The only file the server opens. | required |
| `--read-only` | Hide and refuse the write tools; no transaction is started. | off |
| `--timeout-ms <n>` | Deadline of one tool call (`DeadlineExceeded`). | 30000 |
| `--reader-timeout-ms <n>` | Wait for a read connection (`PoolTimeout`). | unbounded |
| `--max-rows <n>` | Rows one call may decode (`ResultLimitExceeded`). | 10000 |
| `--max-bytes <n>` | Decoded bytes one call may produce (`ResultLimitExceeded`). | 8388608 |
| `--text-index` | Build the derived text index at open, for `text_search`. | off |

`0` turns a bound off. The bounds apply to every tool call as one `QueryBudget`; a call past one fails with a structured error, commits nothing, and the server goes on serving.

## Tools

Arguments use the JSON forms of the tiramemsu bindings. A term is a string (a plain string literal), a number, a boolean, or an object: `{"iri": "urn:tiramemsu:v:alice"}` (which SPARQL and Cypher write `v:alice`), `{"stmt": 7}`, `{"node": 3}`, `{"lex": "2024-01-01", "datatype": "http://www.w3.org/2001/XMLSchema#date"}`. A view is `{"kind": "now" | "asOf" | "history", "tx": n, "instant": time, "validAt": time}` and defaults to now.

| Tool | Writes | Arguments | Returns |
|---|---|---|---|
| `assert` | yes | `s`, `p`, `o`, `validFrom`, `validTo`, `onExisting` (`return` or `confirm`), `graph` | `{eid, new, t}`; a repeated fact returns the existing eid with `new: false` |
| `confirm` | yes | `eid` | `{eid, confirmation, t}` |
| `supersede` | yes | `eid`, `patch` (`o`, `validFrom`, `validTo`) | `{old, eid, t, retracted}` |
| `import_bundle` | yes | `bundle` (`tiramemsu-bundle/1`) | `{root, statements, t}` |
| `query` | no | `language` (`sparql` or `cypher`), `text`, `params` (Cypher), `view`, `provenance` | `{language, view, result, provenance}` |
| `dependents` | no | `eid`, `view` | `{eid, view, dependents}` |
| `export_bundle` | no | `eid`, `view` | `{view, bundle}` |
| `conflicts` | no | `s`, `p`, `limit`, `confidence`, `source`, `view` | `{view, conflicts}`: overlapping distinct values, each statement with its evidence |
| `preview_bundle` | no | `bundle` | `{preview}`: proposed and reused statements, dependency changes, `failure`, burned ids, `scope` |
| `text_search` | no | `text`, `mode`, `graphs`, `predicates`, `limit`, `confidence`, `view` | `{view, hits}` with evidence per hit |
| `save_answer` | answer record | `name`, `language`, `text`, `params`, `view` | `{answer}`: result, cited statements, coverage, status |
| `saved_answers` | no | `name` (optional) | `{answers}` |
| `check_answers` | answer records | none | `{invalidations}`: each `stale` or `recheck` mark once, with its event |
| `refresh_answer` | answer records | `name` | `{answer}`; only success makes it fresh again |

- **Queries only read.** A SPARQL update or a Cypher write clause is refused with `Unsupported` before anything runs; there is no raw SQL.
- **Provenance coverage.** A SPARQL `SELECT` lists the statements behind each row unless `provenance` is `false`. `provenance.coverage` is `complete`, `incomplete` (with `gaps`, e.g. `["recursivePath"]` for a `+` or `*` path, whose endpoints carry no statement ids) or `unavailable` (Cypher, `ASK`, `CONSTRUCT`, or not requested).
- **Saved answers.** `save_answer`, `check_answers` and `refresh_answer` write only the derived answer records, never facts, but are left out in read-only mode with the other writing tools.
- **Conflict review.** `conflicts` never retracts, supersedes or confirms, and refuses the history view. `preview_bundle` runs the import as a dry run on the writer: graph, history and event log stay unchanged and only the ids it allocated are burned, so it is offered in read-only mode. Applying is `import_bundle`, which validates again.
- **No other file.** An argument such as `path`, `db` or `database` is refused with `PathNotAllowed`, and any argument a tool does not declare with `InvalidArgument`.

## Errors

A failed call is a tool result with `isError: true` whose content is `{"code", "message"}`. Codes are the JSON bridge's (`InvalidArgument`, `Parse`, `Unsupported`, `NotLive`, `InvalidPatch`, `DeadlineExceeded`, `ResultLimitExceeded`, `TextIndexUnavailable`, ...) plus `ReadOnly` and `PathNotAllowed`. Protocol errors (malformed JSON-RPC, an unknown method or tool) are JSON-RPC errors.

## Protocol

JSON-RPC 2.0, one message per line on stdin and stdout; diagnostics go to stderr. Methods: `initialize`, `ping`, `tools/list`, `tools/call`; notifications are accepted and need no answer. The server speaks revision `2025-06-18` and accepts `2025-03-26` and `2024-11-05`; tool results carry `structuredContent` from `2025-06-18` on.

## As a library

`Server` handles one message at a time with no transport of its own, so it can be embedded or tested without a process:

```rust
use serde_json::{json, Value};
use tiramemsu_mcp::{Config, Server};

# let dir = tempfile::tempdir().unwrap();
let mut server = Server::open(Config::new(dir.path().join("memory.db")))?;
let call = |name: &str, args: Value| {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": args } })
    .to_string()
};
let alice = json!({ "iri": "urn:tiramemsu:v:alice" });
let works_at = json!({ "iri": "urn:tiramemsu:v:worksAt" });
let acme = json!({ "iri": "urn:tiramemsu:v:acme" });
let out = server.handle(&call("assert", json!({ "s": alice, "p": works_at, "o": acme }))).unwrap();
let out: Value = serde_json::from_str(&out).unwrap();
assert_eq!(out["result"]["structuredContent"]["new"], true);

let q = "SELECT ?o WHERE { v:alice v:worksAt ?o }";
let r = server.call_tool("query", &json!({ "language": "sparql", "text": q }))?;
assert_eq!(r["result"]["rows"], json!([{ "o": acme }]));
assert_eq!(r["provenance"]["coverage"], "complete");
assert_eq!(r["view"], json!({ "kind": "now" }));
# Ok::<(), tiramemsu_mcp::ToolError>(())
```

## License

MIT OR Apache-2.0.
