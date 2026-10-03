//! The `tiramemsu-mcp` binary over stdio, as an MCP client starts it.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value as J};

const BIN: &str = env!("CARGO_BIN_EXE_tiramemsu-mcp");

// a whole session: handshake, write, read, an error, and a clean exit at EOF
// @lat: [[tests#MCP Adapter#Stdio Session End To End]]
#[test]
fn stdio_session_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memory.db");
    let mut child = Command::new(BIN)
        .args(["--db", db.to_str().unwrap(), "--text-index"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    // writes one message (a JSON string is sent as raw text) and reads the
    // answer when one is due
    let mut ask = move |msg: J| -> Option<J> {
        let (text, answered) = match &msg {
            J::String(raw) => (raw.clone(), true),
            m => (m.to_string(), m.get("id").is_some()),
        };
        writeln!(stdin, "{text}").unwrap();
        stdin.flush().unwrap();
        if !answered {
            return None;
        }
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        Some(serde_json::from_str(&line).unwrap())
    };
    let r = ask(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": { "name": "test", "version": "1" } } }))
    .unwrap();
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert!(ask(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
    let r = ask(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).unwrap();
    let names: Vec<&str> = r["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "assert",
            "confirm",
            "supersede",
            "query",
            "dependents",
            "export_bundle",
            "import_bundle",
            "text_search"
        ]
    );
    let call = |id: u64, name: &str, args: J| {
        json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": { "name": name, "arguments": args } })
    };
    let alice = json!({ "iri": "urn:tiramemsu:v:alice" });
    let r = ask(call(
        3,
        "assert",
        json!({ "s": alice, "p": { "iri": "urn:tiramemsu:v:note" }, "o": "met at the Lisbon offsite" }),
    ))
    .unwrap();
    assert_eq!(r["result"]["structuredContent"]["new"], true);
    // a malformed request is answered, and the server keeps serving
    let err = ask(json!("{oops")).unwrap();
    assert_eq!(err["error"]["code"], -32700);
    let r = ask(call(
        4,
        "query",
        json!({ "language": "sparql", "text": "SELECT ?" }),
    ))
    .unwrap();
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(r["result"]["structuredContent"]["code"], "Parse");
    let r = ask(call(
        5,
        "query",
        json!({ "language": "sparql", "text": "SELECT ?o WHERE { v:alice v:note ?o }" }),
    ))
    .unwrap();
    let out = &r["result"]["structuredContent"];
    assert_eq!(
        out["result"]["rows"],
        json!([{ "o": "met at the Lisbon offsite" }])
    );
    assert_eq!(out["provenance"]["coverage"], "complete");
    let r = ask(call(6, "text_search", json!({ "text": "lisbon" }))).unwrap();
    assert_eq!(r["result"]["structuredContent"]["hits"][0]["s"], alice);
    drop(ask); // closes stdin
    let status = child.wait().unwrap();
    assert!(status.success());
}

// the binary refuses to start without --db, and explains itself on stderr
#[test]
fn the_binary_needs_a_database() {
    let out = Command::new(BIN).arg("--read-only").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--db"));
    let help = Command::new(BIN).arg("--help").output().unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("claude mcp add"));
}
