#![doc = include_str!("../README.md")]
// @lat: [[api#MCP Tools]]

mod config;
pub mod tools;

use std::io::{BufRead, Write};

use serde_json::{json, Value as J};
use tiramemsu_json::Database;

pub use config::{
    parse_args, Budget, Command, Config, DEFAULT_MAX_BYTES, DEFAULT_MAX_ROWS, DEFAULT_TIMEOUT_MS,
    USAGE,
};
pub use tools::ToolError;

/// The MCP protocol revision this server implements.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// Every revision a client may ask for; any other request gets
/// [`PROTOCOL_VERSION`], and the client decides whether to go on.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

/// JSON-RPC: the message is not JSON.
pub const PARSE_ERROR: i64 = -32700;
/// JSON-RPC: the message is not a request.
pub const INVALID_REQUEST: i64 = -32600;
/// JSON-RPC: the method does not exist.
pub const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC: the parameters are malformed (also an unknown tool).
pub const INVALID_PARAMS: i64 = -32602;

const INSTRUCTIONS: &str = "Tiramemsu is a bitemporal, never-forget memory: every fact is a \
statement (s, p, o) with its own id (eid), and an eid can be the subject or object of another \
statement, so sources, confidence and beliefs are layers on facts. Nothing is deleted: supersede \
corrects a fact and keeps the old one visible in asOf and history views. Terms are JSON: \
{\"iri\": \"urn:tiramemsu:v:alice\"} is v:alice in SPARQL and Cypher, a plain string is a string \
literal, and {\"stmt\": eid} names a statement. Use query for SPARQL or Cypher reads, text_search \
for recall by words, conflicts to see disagreeing values with their evidence, preview_bundle to \
see what an import would do, and assert, confirm, supersede and import_bundle to write. save_answer keeps \
an answer; check_answers says when it is stale or needs a recheck, and refresh_answer re-runs it.";

/// A memory server over one database file: a JSON-RPC 2.0 message in, a message
/// out, with no transport of its own. [`Server::serve`] drives it over lines of
/// a reader and a writer (stdio for the binary).
#[derive(Debug)]
pub struct Server {
    db: Database,
    config: Config,
    /// The negotiated protocol revision; the newest until a client initializes.
    protocol: String,
}

impl Server {
    /// Opens (creating if needed) the configured database.
    ///
    /// # Errors
    ///
    /// The bridge's error when the file cannot be opened, such as `ForeignFile`,
    /// `FormatVersion` or `Sqlite`.
    pub fn open(config: Config) -> Result<Server, ToolError> {
        let path = config
            .db
            .to_str()
            .ok_or_else(|| ToolError::new("InvalidArgument", "the database path is not UTF-8"))?;
        let db = Database::open(path, &json!({ "textIndex": config.text_index }))?;
        Ok(Server {
            db,
            config,
            protocol: PROTOCOL_VERSION.to_string(),
        })
    }

    /// The configuration the server runs with.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The MCP definitions of the tools this server offers (`tools/list`).
    pub fn tools(&self) -> Vec<J> {
        tools::definitions(&self.config)
    }

    /// Runs one tool and returns its structured result.
    ///
    /// # Errors
    ///
    /// The tool's [`ToolError`]; an unknown tool is `UnknownTool`.
    pub fn call_tool(&self, name: &str, args: &J) -> Result<J, ToolError> {
        tools::call(&self.db, &self.config, name, args)
            .unwrap_or_else(|| Err(ToolError::new("UnknownTool", format!("no tool {name:?}"))))
    }

    /// Handles one line of JSON-RPC text: a message or a batch. Returns the
    /// response text, or `None` when there is nothing to answer (notifications,
    /// responses, blank lines).
    pub fn handle(&mut self, line: &str) -> Option<String> {
        if line.trim().is_empty() {
            return None;
        }
        let out = match serde_json::from_str::<J>(line) {
            Ok(msg) => self.handle_value(msg)?,
            Err(e) => error_response(J::Null, PARSE_ERROR, &format!("not JSON: {e}")),
        };
        Some(out.to_string())
    }

    /// [`Server::handle`] on a parsed message or batch.
    pub fn handle_value(&mut self, msg: J) -> Option<J> {
        match msg {
            J::Array(batch) if batch.is_empty() => {
                Some(error_response(J::Null, INVALID_REQUEST, "an empty batch"))
            }
            J::Array(batch) => {
                let out: Vec<J> = batch.into_iter().filter_map(|m| self.message(m)).collect();
                (!out.is_empty()).then_some(J::Array(out))
            }
            m => self.message(m),
        }
    }

    /// One message: a request gets a response, a notification or a response from
    /// the client gets nothing.
    fn message(&mut self, msg: J) -> Option<J> {
        let J::Object(o) = &msg else {
            return Some(error_response(
                J::Null,
                INVALID_REQUEST,
                "a message is an object",
            ));
        };
        let id = o.get("id").cloned();
        if o.get("jsonrpc").and_then(J::as_str) != Some("2.0") {
            return Some(error_response(
                id.unwrap_or(J::Null),
                INVALID_REQUEST,
                "jsonrpc must be \"2.0\"",
            ));
        }
        let Some(method) = o.get("method").and_then(J::as_str) else {
            if o.contains_key("result") || o.contains_key("error") {
                return None; // a response; this server sends no requests
            }
            return Some(error_response(
                id.unwrap_or(J::Null),
                INVALID_REQUEST,
                "a request needs a method",
            ));
        };
        let params = o.get("params").cloned().unwrap_or(J::Null);
        let Some(id) = id else {
            // notifications (initialized, cancelled, ...) need no answer
            return None;
        };
        if !matches!(id, J::String(_) | J::Number(_)) {
            return Some(error_response(
                J::Null,
                INVALID_REQUEST,
                "id must be a string or a number",
            ));
        }
        Some(match self.request(method, &params) {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => error_response(id, code, &message),
        })
    }

    fn request(&mut self, method: &str, params: &J) -> Result<J, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(J::as_str);
                self.protocol = match asked {
                    Some(v) if SUPPORTED_PROTOCOL_VERSIONS.contains(&v) => v.to_string(),
                    _ => PROTOCOL_VERSION.to_string(),
                };
                Ok(json!({
                    "protocolVersion": self.protocol,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": {
                        "name": "tiramemsu-mcp",
                        "title": "Tiramemsu memory",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.tools() })),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(J::as_str)
                    .ok_or((INVALID_PARAMS, "tools/call needs a tool name".to_string()))?;
                let args = params.get("arguments").unwrap_or(&J::Null);
                match tools::call(&self.db, &self.config, name, args) {
                    None => Err((INVALID_PARAMS, format!("unknown tool {name:?}"))),
                    Some(r) => Ok(self.tool_result(r)),
                }
            }
            other => Err((METHOD_NOT_FOUND, format!("method {other:?} not found"))),
        }
    }

    /// A tool outcome as an MCP tool result: the JSON as text, plus
    /// `structuredContent` from revision 2025-06-18 on; a failure sets `isError`.
    fn tool_result(&self, r: Result<J, ToolError>) -> J {
        let (body, is_error) = match r {
            Ok(j) => (j, false),
            Err(e) => (e.to_json(), true),
        };
        let mut out = json!({
            "content": [{ "type": "text", "text": body.to_string() }],
            "isError": is_error,
        });
        if self.protocol.as_str() >= "2025-06-18" {
            out["structuredContent"] = body;
        }
        out
    }

    /// Serves newline-delimited JSON-RPC from `input` to `output` until `input`
    /// ends. A failing request is answered with an error and the next one is
    /// served; only an I/O error on the streams stops the loop.
    ///
    /// # Errors
    ///
    /// The I/O error that ended the loop.
    pub fn serve(&mut self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            if let Some(out) = self.handle(&line?) {
                writeln!(output, "{out}")?;
                output.flush()?;
            }
        }
        Ok(())
    }
}

fn error_response(id: J, code: i64, message: &str) -> J {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
