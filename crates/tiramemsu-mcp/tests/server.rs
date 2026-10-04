//! The memory tools in process: every scenario of `mcp-memory-adapter`, through
//! `Server::handle` (the JSON-RPC layer) and `Server::call_tool`.

use serde_json::{json, Value as J};
use tiramemsu_mcp::{Budget, Config, Server};

fn v(s: &str) -> J {
    json!({ "iri": format!("urn:tiramemsu:v:{s}") })
}

fn config(dir: &tempfile::TempDir) -> Config {
    Config::new(dir.path().join("memory.db"))
}

fn open(dir: &tempfile::TempDir) -> Server {
    Server::open(config(dir)).unwrap()
}

/// A `tools/call` request through the JSON-RPC layer; returns the tool result.
fn rpc_call(server: &mut Server, name: &str, args: J) -> J {
    let req = json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call",
                      "params": { "name": name, "arguments": args } });
    let out: J = serde_json::from_str(&server.handle(&req.to_string()).unwrap()).unwrap();
    assert_eq!(out["id"], 9);
    out["result"].clone()
}

/// The error code of a failed call, checking that it is a tool result with
/// `isError` and the same `{code, message}` as text and structured content.
fn tool_error(result: &J) -> String {
    assert_eq!(result["isError"], true, "{result}");
    let text: J = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, result["structuredContent"]);
    assert!(text["message"].as_str().is_some_and(|m| !m.is_empty()));
    text["code"].as_str().unwrap().to_string()
}

/// The event count and the last transaction: what a refused write must not move.
fn counters(server: &Server) -> (usize, J) {
    let r = server
        .call_tool(
            "query",
            &json!({ "language": "sparql", "view": { "kind": "history" },
                     "text": "SELECT ?s ?p ?o WHERE { ?s ?p ?o }", "provenance": false }),
        )
        .unwrap();
    let rows = r["result"]["rows"].as_array().unwrap().len();
    let t = server
        .call_tool(
            "query",
            &json!({ "language": "sparql", "provenance": false,
                     "text": "SELECT (MAX(?t) AS ?last) WHERE { ?e tm:txAdded ?t }",
                     "view": { "kind": "history" } }),
        )
        .unwrap();
    (rows, t["result"]["rows"].clone())
}

fn seed(server: &Server) -> u64 {
    let r = server
        .call_tool(
            "assert",
            &json!({ "s": v("alice"), "p": v("worksAt"), "o": v("acme") }),
        )
        .unwrap();
    r["eid"].as_u64().unwrap()
}

// mcp-memory-adapter "Read-only mutation"
// @lat: [[tests#MCP Adapter#Read Only Refuses Writes Before A Transaction]]
#[test]
fn read_only_refuses_writes_before_a_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let eid = seed(&open(&dir));
    let mut ro = Server::open(Config {
        read_only: true,
        ..config(&dir)
    })
    .unwrap();
    let before = counters(&ro);
    let r = rpc_call(
        &mut ro,
        "supersede",
        json!({ "eid": eid, "patch": { "o": v("globex") } }),
    );
    assert_eq!(tool_error(&r), "ReadOnly");
    // refused before the arguments are even looked at
    for (name, args) in [
        ("assert", json!({ "s": "x" })),
        ("confirm", json!({ "eid": eid })),
        ("import_bundle", json!({ "bundle": 1 })),
    ] {
        assert_eq!(tool_error(&rpc_call(&mut ro, name, args)), "ReadOnly");
    }
    assert_eq!(counters(&ro), before);
    // the write tools are not offered, the reads still work
    let names: Vec<String> = ro
        .tools()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "query",
            "dependents",
            "export_bundle",
            "conflicts",
            "preview_bundle",
            "text_search",
            "saved_answers"
        ]
    );
    for name in ["save_answer", "check_answers", "refresh_answer"] {
        assert_eq!(tool_error(&rpc_call(&mut ro, name, json!({}))), "ReadOnly");
    }
    let r = ro.call_tool("dependents", &json!({ "eid": eid })).unwrap();
    // a statement stands on itself: retracting it retracts it
    assert_eq!(r["dependents"], json!([eid]));
}

// mcp-memory-adapter "Path injection"
// @lat: [[tests#MCP Adapter#Requests Cannot Name Another Database]]
#[test]
fn requests_cannot_name_another_database() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = open(&dir);
    let other = dir.path().join("other.db");
    let other_s = other.to_str().unwrap();
    for (name, args) in [
        (
            "query",
            json!({ "language": "sparql", "text": "ASK {}", "path": other_s }),
        ),
        (
            "assert",
            json!({ "s": v("a"), "p": v("p"), "o": v("b"), "db": other_s }),
        ),
        ("text_search", json!({ "text": "x", "database": other_s })),
        ("export_bundle", json!({ "eid": 1, "file": other_s })),
    ] {
        assert_eq!(
            tool_error(&rpc_call(&mut server, name, args)),
            "PathNotAllowed",
            "{name}"
        );
    }
    // any undeclared argument is refused too, and so is SQL
    let r = rpc_call(
        &mut server,
        "query",
        json!({ "language": "sparql", "text": "ASK {}", "budget": { "maxRows": 0 } }),
    );
    assert_eq!(tool_error(&r), "InvalidArgument");
    let r = rpc_call(
        &mut server,
        "query",
        json!({ "language": "sql", "text": "SELECT * FROM triple" }),
    );
    assert_eq!(tool_error(&r), "InvalidArgument");
    assert!(!other.exists());
    // the configured file holds nothing from the refused calls
    assert_eq!(counters(&server).0, 0);
}

// mcp-memory-adapter "Repeated assertion"
// @lat: [[tests#MCP Adapter#Repeated Assert Returns The Existing Statement]]
#[test]
fn repeated_assert_returns_the_existing_statement() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = open(&dir);
    let fact = json!({ "s": v("alice"), "p": v("worksAt"), "o": v("acme"),
                       "validFrom": "2020-01-01" });
    let first = rpc_call(&mut server, "assert", fact.clone());
    assert_eq!(first["isError"], false);
    let first = first["structuredContent"].clone();
    assert_eq!(first["new"], true);
    // the same fact with overlapping valid time
    let mut again = fact.clone();
    again["validFrom"] = json!("2021-06-01");
    let second = rpc_call(&mut server, "assert", again)["structuredContent"].clone();
    assert_eq!(second["eid"], first["eid"]);
    assert_eq!(second["new"], false);
    // confirm and supersede use the existing semantics
    let c = server
        .call_tool("confirm", &json!({ "eid": first["eid"] }))
        .unwrap();
    assert_eq!(c["eid"], first["eid"]);
    assert!(c["confirmation"].as_u64().is_some());
    let s = server
        .call_tool(
            "supersede",
            &json!({ "eid": { "stmt": first["eid"] }, "patch": { "o": v("globex") } }),
        )
        .unwrap();
    assert_ne!(s["eid"], first["eid"]);
    let r = server
        .call_tool(
            "query",
            &json!({ "language": "cypher",
                     "text": "MATCH (:Resource {iri: 'urn:tiramemsu:v:alice'})-[r:worksAt]->(o) RETURN o.iri AS o" }),
        )
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r["provenance"]["coverage"], "unavailable");
    // the old statement is gone from now but not forgotten
    let e = server
        .call_tool(
            "supersede",
            &json!({ "eid": first["eid"], "patch": { "o": v("x") } }),
        )
        .unwrap_err();
    assert_eq!(e.code, "NotLive");
    let then = json!({ "kind": "asOf", "tx": c["t"] });
    let old = server
        .call_tool("dependents", &json!({ "eid": first["eid"], "view": then }))
        .unwrap();
    assert_eq!(old["view"], then);
    // the statement and its confirmation
    assert_eq!(old["dependents"], json!([first["eid"], c["confirmation"]]));
    // a graph adds a membership in the same transaction
    let g = server
        .call_tool(
            "assert",
            &json!({ "s": v("bob"), "p": v("knows"), "o": v("alice"), "graph": v("chat1") }),
        )
        .unwrap();
    assert_eq!(g["membership"]["new"], true);
}

// mcp-memory-adapter "Invalid bundle"
// @lat: [[tests#MCP Adapter#Malformed Bundle Commits Nothing]]
#[test]
fn malformed_bundle_commits_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = open(&dir);
    let eid = seed(&server);
    server
        .call_tool(
            "assert",
            &json!({ "s": { "stmt": eid }, "p": v("source"), "o": "chat-1" }),
        )
        .unwrap();
    let bundle = server
        .call_tool("export_bundle", &json!({ "eid": eid }))
        .unwrap()["bundle"]
        .clone();
    assert_eq!(bundle["format"], "tiramemsu-bundle/1");
    let before = counters(&server);
    let mut broken = bundle.clone();
    broken["statements"][0]["s"] = json!(null);
    for bad in [json!({ "format": "other/1" }), json!("text"), broken] {
        let r = rpc_call(&mut server, "import_bundle", json!({ "bundle": bad }));
        assert_eq!(tool_error(&r), "InvalidArgument");
    }
    assert_eq!(counters(&server), before);
    // the bundle itself imports into a fresh memory
    let dir2 = tempfile::tempdir().unwrap();
    let fresh = open(&dir2);
    let r = fresh
        .call_tool("import_bundle", &json!({ "bundle": bundle }))
        .unwrap();
    assert_eq!(r["statements"].as_array().unwrap().len(), 2);
    assert_eq!(r["t"], 1);
}

// mcp-memory-adapter "Incomplete provenance"
// @lat: [[tests#MCP Adapter#Query Reports View And Provenance Coverage]]
#[test]
fn query_reports_view_and_provenance_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let server = open(&dir);
    for (a, b) in [("a", "b"), ("b", "c")] {
        server
            .call_tool("assert", &json!({ "s": v(a), "p": v("knows"), "o": v(b) }))
            .unwrap();
    }
    let q = |text: &str, extra: J| {
        let mut args = json!({ "language": "sparql", "text": text });
        for (k, val) in extra.as_object().unwrap() {
            args[k] = val.clone();
        }
        server.call_tool("query", &args).unwrap()
    };
    let r = q("SELECT ?x WHERE { v:a v:knows+ ?x }", json!({}));
    assert_eq!(r["result"]["rows"].as_array().unwrap().len(), 2);
    assert_eq!(r["result"]["provenance"], json!([[], []]));
    assert_eq!(r["provenance"]["coverage"], "incomplete");
    assert_eq!(r["provenance"]["gaps"], json!(["recursivePath"]));
    assert_eq!(r["view"], json!({ "kind": "now" }));
    let r = q(
        "PREFIX ex: <http://example.org/#>\nSELECT ?x WHERE { v:a v:knows ?x }",
        json!({ "view": { "kind": "asOf", "tx": 1 } }),
    );
    assert_eq!(r["result"]["provenance"], json!([[{ "stmt": 1 }]]));
    assert_eq!(
        r["provenance"],
        json!({ "coverage": "complete", "gaps": [] })
    );
    assert_eq!(r["view"], json!({ "kind": "asOf", "tx": 1 }));
    let r = q("ASK { v:a v:knows v:b }", json!({}));
    assert_eq!(r["result"]["value"], true);
    assert_eq!(r["provenance"]["coverage"], "unavailable");
    let r = q(
        "SELECT ?x WHERE { v:a v:knows ?x }",
        json!({ "provenance": false }),
    );
    assert_eq!(r["provenance"]["coverage"], "unavailable");
    assert!(r["result"].get("provenance").is_none());
    // an unsupported combination is an error, not retried as something else
    let e = server
        .call_tool(
            "query",
            &json!({ "language": "sparql", "text": "ASK {}", "provenance": true }),
        )
        .unwrap_err();
    assert_eq!(e.code, "Unsupported");
    let e = server
        .call_tool(
            "query",
            &json!({ "language": "cypher", "text": "MATCH (n) RETURN n", "provenance": true }),
        )
        .unwrap_err();
    assert_eq!(e.code, "InvalidArgument");
}

// queries only read: a SPARQL update or a Cypher write is refused
// @lat: [[tests#MCP Adapter#Queries Cannot Write]]
#[test]
fn queries_cannot_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = open(&dir);
    for (language, text) in [
        ("sparql", "INSERT DATA { v:a v:p v:b }"),
        ("sparql", "PREFIX ex: <http://x/> DELETE WHERE { ?s ?p ?o }"),
        ("cypher", "CREATE (:Person {name: 'Eve'})"),
    ] {
        let r = rpc_call(
            &mut server,
            "query",
            json!({ "language": language, "text": text }),
        );
        assert_eq!(tool_error(&r), "Unsupported", "{text}");
    }
    assert_eq!(counters(&server).0, 0);
}

// mcp-memory-adapter "Limit exceeded"
// @lat: [[tests#MCP Adapter#Budget Errors Leave The Server Serving]]
#[test]
fn budget_errors_leave_the_server_serving() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = Server::open(Config {
        budget: Budget {
            max_rows: Some(3),
            ..Budget::default()
        },
        ..config(&dir)
    })
    .unwrap();
    for o in ["a", "b", "c", "d", "e"] {
        server
            .call_tool("assert", &json!({ "s": v("x"), "p": v("p"), "o": v(o) }))
            .unwrap();
    }
    let all = json!({ "language": "sparql", "text": "SELECT ?o WHERE { v:x v:p ?o }" });
    let r = rpc_call(&mut server, "query", all);
    assert_eq!(tool_error(&r), "ResultLimitExceeded");
    // the next request runs
    let r = rpc_call(
        &mut server,
        "query",
        json!({ "language": "sparql", "text": "SELECT ?o WHERE { v:x v:p ?o } LIMIT 2",
                "provenance": false }),
    );
    assert_eq!(r["isError"], false);
    assert_eq!(
        r["structuredContent"]["result"]["rows"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // an expired deadline is structured too
    let slow = Server::open(Config {
        budget: Budget {
            timeout_ms: Some(1),
            ..Budget::default()
        },
        ..config(&dir)
    })
    .unwrap();
    let deep = "SELECT (COUNT(*) AS ?n) WHERE { ?a ?p ?b . ?c ?q ?d . ?e ?r ?f . ?g ?s ?h . ?i ?t ?j . ?k ?u ?l }";
    let mut hit = false;
    for _ in 0..20 {
        match slow.call_tool(
            "query",
            &json!({ "language": "sparql", "text": deep, "provenance": false }),
        ) {
            Err(e) => {
                assert_eq!(e.code, "DeadlineExceeded");
                hit = true;
                break;
            }
            Ok(_) => continue,
        }
    }
    assert!(hit, "a 1 ms deadline never expired");
    assert!(slow.call_tool("dependents", &json!({ "eid": 1 })).is_ok());
    // text search without the index is a structured error as well
    let e = server
        .call_tool("text_search", &json!({ "text": "acme" }))
        .unwrap_err();
    assert_eq!(e.code, "TextIndexUnavailable");
}

// text recall through the adapter, on a server that builds the index at open
#[test]
fn text_search_returns_ranked_hits_with_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let server = Server::open(Config {
        text_index: true,
        ..config(&dir)
    })
    .unwrap();
    server
        .call_tool(
            "assert",
            &json!({ "s": v("trip"), "p": v("note"), "o": "Lisbon offsite in May" }),
        )
        .unwrap();
    let r = server
        .call_tool("text_search", &json!({ "text": "lisbon", "limit": 5 }))
        .unwrap();
    assert_eq!(r["view"], json!({ "kind": "now" }));
    let hits = r["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["rank"], 1);
    assert_eq!(hits[0]["o"], "Lisbon offsite in May");
    assert!(hits[0]["evidence"].is_object());
    // the write tool's graph adds a membership; the search tool's graphs filter by it
    server
        .call_tool(
            "assert",
            &json!({ "s": v("call"), "p": v("note"), "o": "Lisbon flights booked", "graph": v("chat1") }),
        )
        .unwrap();
    let all = server
        .call_tool("text_search", &json!({ "text": "lisbon" }))
        .unwrap();
    assert_eq!(all["hits"].as_array().unwrap().len(), 2);
    let scoped = server
        .call_tool(
            "text_search",
            &json!({ "text": "lisbon", "graphs": [v("chat1")] }),
        )
        .unwrap();
    let scoped = scoped["hits"].as_array().unwrap();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0]["o"], "Lisbon flights booked");
}

// the JSON-RPC layer: handshake, negotiation, ping, listing and protocol errors
// @lat: [[tests#MCP Adapter#Protocol Handshake And Errors]]
#[test]
fn protocol_handshake_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = open(&dir);
    let send = |server: &mut Server, msg: J| -> Option<J> {
        server
            .handle(&msg.to_string())
            .map(|s| serde_json::from_str(&s).unwrap())
    };
    // an older revision is accepted, an unknown one gets the newest
    let init = |v: &str| {
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": v, "capabilities": {},
                    "clientInfo": { "name": "t", "version": "0" } } })
    };
    let r = send(&mut server, init("2024-11-05")).unwrap();
    assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(r["result"]["serverInfo"]["name"], "tiramemsu-mcp");
    assert_eq!(r["result"]["capabilities"]["tools"]["listChanged"], false);
    // before 2025-06-18 results carry text content only
    let call = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "dependents", "arguments": { "eid": 1 } } });
    let r = send(&mut server, call.clone()).unwrap();
    assert!(r["result"].get("structuredContent").is_none());
    assert_eq!(r["result"]["content"][0]["type"], "text");
    let r = send(&mut server, init("1999-01-01")).unwrap();
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    let r = send(&mut server, call).unwrap();
    assert_eq!(r["result"]["structuredContent"]["dependents"], json!([]));
    // notifications and client responses get no answer
    assert!(send(
        &mut server,
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })
    )
    .is_none());
    assert!(send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 5, "result": {} })
    )
    .is_none());
    assert!(server.handle("   ").is_none());
    let r = send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": "p", "method": "ping" }),
    )
    .unwrap();
    assert_eq!(r, json!({ "jsonrpc": "2.0", "id": "p", "result": {} }));
    let r = send(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }),
    )
    .unwrap();
    assert_eq!(r["result"]["tools"].as_array().unwrap().len(), 14);
    // protocol errors
    let code = |r: Option<J>| r.unwrap()["error"]["code"].as_i64().unwrap();
    let raw: J = serde_json::from_str(&server.handle("{not json").unwrap()).unwrap();
    assert_eq!(raw["error"]["code"], -32700);
    assert_eq!(raw["id"], J::Null);
    assert_eq!(
        code(send(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 4, "method": "resources/list" })
        )),
        -32601
    );
    assert_eq!(
        code(send(
            &mut server,
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": { "name": "drop_everything" } })
        )),
        -32602
    );
    assert_eq!(
        code(send(&mut server, json!({ "id": 4, "method": "ping" }))),
        -32600
    );
    assert_eq!(code(send(&mut server, json!([]))), -32600);
    // a batch answers each request, and the server goes on
    let r = send(
        &mut server,
        json!([{ "jsonrpc": "2.0", "id": 7, "method": "ping" },
               { "jsonrpc": "2.0", "method": "notifications/initialized" },
               { "jsonrpc": "2.0", "id": 8, "method": "nope" }]),
    )
    .unwrap();
    assert_eq!(r.as_array().unwrap().len(), 2);
    assert_eq!(r[1]["error"]["code"], -32601);
    assert_eq!(
        server.call_tool("nope", &J::Null).unwrap_err().code,
        "UnknownTool"
    );
}

// @lat: [[tests#Saved Answers#MCP Saved Answer Tools]]
#[test]
fn saved_answer_tools_mark_and_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let server = open(&dir);
    let eid = seed(&server);
    let saved = server
        .call_tool(
            "save_answer",
            &json!({ "name": "employer", "language": "sparql",
                     "text": "SELECT ?o WHERE { v:alice v:worksAt ?o }" }),
        )
        .unwrap();
    assert_eq!(saved["answer"]["status"], "fresh");
    assert_eq!(saved["answer"]["dependencies"], json!([eid]));
    // an update is not an answer
    let r = server.call_tool(
        "save_answer",
        &json!({ "name": "w", "language": "sparql", "text": "INSERT DATA { v:a v:p v:b }" }),
    );
    assert_eq!(r.unwrap_err().code, "Unsupported");
    server
        .call_tool(
            "supersede",
            &json!({ "eid": eid, "patch": { "o": v("globex") } }),
        )
        .unwrap();
    let marks = server.call_tool("check_answers", &json!({})).unwrap();
    assert_eq!(marks["invalidations"][0]["status"], "stale");
    assert_eq!(marks["invalidations"][0]["cause"], "supportRetracted");
    let again = server.call_tool("check_answers", &json!({})).unwrap();
    assert_eq!(again["invalidations"], json!([]));
    let read = server
        .call_tool("saved_answers", &json!({ "name": "employer" }))
        .unwrap();
    assert_eq!(read["answers"][0]["status"], "stale");
    let fresh = server
        .call_tool("refresh_answer", &json!({ "name": "employer" }))
        .unwrap();
    assert_eq!(fresh["answer"]["status"], "fresh");
    assert_eq!(fresh["answer"]["result"]["rows"][0]["o"], v("globex"));
    let all = server.call_tool("saved_answers", &json!({})).unwrap();
    assert_eq!(all["answers"].as_array().unwrap().len(), 1);
    let missing = server.call_tool("refresh_answer", &json!({ "name": "nope" }));
    assert_eq!(missing.unwrap_err().code, "SavedAnswerNotFound");
}

// @lat: [[tests#Memory Conflict Review#MCP Conflict And Preview Tools]]
#[test]
fn conflict_and_preview_tools_read_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let server = open(&dir);
    seed(&server);
    server
        .call_tool(
            "assert",
            &json!({ "s": v("alice"), "p": v("worksAt"), "o": v("initech"), "validFrom": 5 }),
        )
        .unwrap();
    let src_dir = tempfile::tempdir().unwrap();
    let src = open(&src_dir);
    let b = src
        .call_tool(
            "assert",
            &json!({ "s": v("bob"), "p": v("worksAt"), "o": v("globex") }),
        )
        .unwrap();
    let bundle = src
        .call_tool("export_bundle", &json!({ "eid": b["eid"] }))
        .unwrap()["bundle"]
        .clone();
    drop(server);
    // both tools are read tools, offered in read-only mode
    let ro = Server::open(Config {
        read_only: true,
        ..config(&dir)
    })
    .unwrap();
    let before = counters(&ro);
    let c = ro.call_tool("conflicts", &json!({})).unwrap();
    assert_eq!(c["view"], json!({ "kind": "now" }));
    let list = c["conflicts"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["s"], v("alice"));
    assert_eq!(
        list[0]["overlaps"],
        json!([{ "validFrom": 5, "validTo": null }])
    );
    assert_eq!(list[0]["values"].as_array().unwrap().len(), 2);
    let filtered = ro
        .call_tool("conflicts", &json!({ "s": v("bob"), "p": v("worksAt") }))
        .unwrap();
    assert_eq!(filtered["conflicts"], json!([]));
    let p = ro
        .call_tool("preview_bundle", &json!({ "bundle": bundle }))
        .unwrap();
    assert_eq!(p["preview"]["wouldCommit"], true, "{p}");
    assert_eq!(p["preview"]["statements"][0]["new"], true);
    assert_eq!(counters(&ro), before);
    // the arguments are checked like every tool's
    let e = ro
        .call_tool("preview_bundle", &json!({ "bundle": { "format": "nope" } }))
        .unwrap_err();
    assert_eq!(e.code, "InvalidArgument");
    let e = ro
        .call_tool("conflicts", &json!({ "path": "/tmp/other.db" }))
        .unwrap_err();
    assert_eq!(e.code, "PathNotAllowed");
    let e = ro
        .call_tool("conflicts", &json!({ "view": { "kind": "history" } }))
        .unwrap_err();
    assert_eq!(e.code, "Unsupported");
    // applying stays an explicit write tool, refused here
    let e = ro
        .call_tool("import_bundle", &json!({ "bundle": bundle }))
        .unwrap_err();
    assert_eq!(e.code, "ReadOnly");
    assert_eq!(counters(&ro), before);
}
