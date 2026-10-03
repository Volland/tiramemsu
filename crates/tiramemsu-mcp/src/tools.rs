//! The memory tools: their schemas and how each one maps onto the JSON bridge.
//!
//! Every tool takes JSON-bridge terms and views ([`tiramemsu_json::value`]) and
//! runs as one bridge call under the configured budget. Write tools are one
//! transaction each. A tool argument can never name a file, carry SQL, or change
//! the budget: the argument object is checked against the tool's keys first.

use serde_json::{json, Map, Value as J};
use tiramemsu::{Bundle, BundleFormat};
use tiramemsu_json::{BindError, Database};

use crate::config::Config;

/// A failed tool call, returned to the client as a tool result with `isError`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError {
    /// A stable code: a JSON-bridge code (`InvalidArgument`, `Parse`,
    /// `Unsupported`, `NotLive`, `DeadlineExceeded`, `ResultLimitExceeded`, ...)
    /// or one of the adapter's own, `ReadOnly` and `PathNotAllowed`.
    pub code: String,
    /// What went wrong, for a person.
    pub message: String,
}

impl ToolError {
    /// An error with `code` and `message`.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> ToolError {
        ToolError {
            code: code.into(),
            message: message.into(),
        }
    }

    fn arg(message: impl Into<String>) -> ToolError {
        ToolError::new("InvalidArgument", message)
    }

    /// The error as `{"code", "message"}`, the form of the bridge's errors.
    pub fn to_json(&self) -> J {
        json!({ "code": self.code, "message": self.message })
    }
}

impl From<BindError> for ToolError {
    fn from(e: BindError) -> ToolError {
        ToolError::new(e.code(), e.to_string())
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolError {}

/// Argument names that would select another file. A request carrying one is
/// refused with `PathNotAllowed`: the configured database is the only target.
const PATH_KEYS: &[&str] = &[
    "path",
    "db",
    "dbPath",
    "db_path",
    "database",
    "databasePath",
    "database_path",
    "file",
    "filename",
    "uri",
    "url",
];

/// One tool: its name, whether it writes, and its MCP definition.
struct Tool {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    writes: bool,
    idempotent: bool,
    /// `(name, schema, required)` per argument.
    args: &'static [(&'static str, Schema, bool)],
}

/// The JSON schema of one argument.
#[derive(Clone, Copy)]
enum Schema {
    Term,
    Eid,
    Time,
    View,
    Text(&'static str),
    Enum(&'static str, &'static [&'static str]),
    Object(&'static str),
    Terms(&'static str),
    Count(&'static str),
    Flag(&'static str),
}

const TERM: &str = "A term: a string (a plain string literal), a number, a boolean, or an object \
{\"iri\": ...}, {\"node\": n}, {\"stmt\": eid}, {\"lex\": ..., \"datatype\": ...} or {\"lex\": ..., \"lang\": ...}. \
Bare names in SPARQL and Cypher resolve to IRIs under urn:tiramemsu:v:, so {\"iri\": \"urn:tiramemsu:v:alice\"} is v:alice.";

impl Schema {
    fn json(self) -> J {
        match self {
            Schema::Term => json!({
                "type": ["string", "number", "boolean", "object"],
                "description": TERM,
            }),
            Schema::Eid => json!({
                "type": ["integer", "object"],
                "description": "A statement id: a number or {\"stmt\": n}.",
            }),
            Schema::Time => json!({
                "type": ["integer", "string"],
                "description": "Epoch milliseconds or an RFC 3339 date or date-time.",
            }),
            Schema::View => json!({
                "type": "object",
                "description": "The view to read: {\"kind\": \"now\" | \"asOf\" | \"history\", \"tx\": n, \
            \"instant\": time, \"validAt\": time}. asOf takes exactly one of tx and instant; validAt may accompany any kind. \
            Default: now.",
                "properties": {
                    "kind": { "type": "string", "enum": ["now", "asOf", "history"] },
                    "tx": { "type": "integer", "minimum": 0 },
                    "instant": { "type": ["integer", "string"] },
                    "validAt": { "type": ["integer", "string"] },
                },
                "additionalProperties": false,
            }),
            Schema::Text(d) => json!({ "type": "string", "description": d }),
            Schema::Enum(d, values) => {
                json!({ "type": "string", "enum": values, "description": d })
            }
            Schema::Object(d) => json!({ "type": "object", "description": d }),
            Schema::Terms(d) => {
                json!({ "type": "array", "items": { "description": TERM }, "description": d })
            }
            Schema::Count(d) => json!({ "type": "integer", "minimum": 0, "description": d }),
            Schema::Flag(d) => json!({ "type": "boolean", "description": d }),
        }
    }
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "assert",
        title: "Assert a fact",
        description: "Assert the fact (s, p, o), optionally with valid time and a graph, in one transaction. \
Asserting a fact that is already live with overlapping valid time returns the existing statement \
(new: false) instead of a duplicate. Returns the statement id (eid) and the transaction (t).",
        writes: true,
        idempotent: true,
        args: &[
            ("s", Schema::Term, true),
            ("p", Schema::Term, true),
            ("o", Schema::Term, true),
            ("validFrom", Schema::Time, false),
            ("validTo", Schema::Time, false),
            (
                "onExisting",
                Schema::Enum(
                    "What a repeated assertion does: \"return\" the existing statement (default) or \
\"confirm\" it (adds a sys:confirmedBy layer).",
                    &["return", "confirm"],
                ),
                false,
            ),
            ("graph", Schema::Term, false),
        ],
    },
    Tool {
        name: "confirm",
        title: "Confirm a statement",
        description: "Record that this transaction corroborates the live statement eid: asserts \
(eid sys:confirmedBy tx). Returns the confirmation statement and the transaction.",
        writes: true,
        idempotent: false,
        args: &[("eid", Schema::Eid, true)],
    },
    Tool {
        name: "supersede",
        title: "Correct a statement",
        description: "Replace the live statement eid by a corrected copy: retracts it, asserts the \
patched statement, and replays the layers that stood on it. Nothing is forgotten; the old statement \
stays visible in asOf and history views. Returns the new eid.",
        writes: true,
        idempotent: false,
        args: &[
            ("eid", Schema::Eid, true),
            (
                "patch",
                Schema::Object(
                    "The fields to change: {\"o\": term, \"validFrom\": time, \"validTo\": time}; null clears a valid-time bound.",
                ),
                true,
            ),
        ],
    },
    Tool {
        name: "query",
        title: "Query memory",
        description: "Run a read-only SPARQL (SELECT, ASK, CONSTRUCT) or openCypher query on a view. \
Updates and write clauses are refused; use the write tools. A SPARQL SELECT lists the statements behind \
each row (provenance) unless provenance is false, and every response reports the view and whether the \
provenance is complete. Results are bounded by the server's budget: add LIMIT to large queries.",
        writes: false,
        idempotent: true,
        args: &[
            (
                "language",
                Schema::Enum("The query language.", &["sparql", "cypher"]),
                true,
            ),
            ("text", Schema::Text("The query text."), true),
            ("params", Schema::Object("Cypher parameters ($name)."), false),
            ("view", Schema::View, false),
            (
                "provenance",
                Schema::Flag("SPARQL only: list the statements behind each SELECT row. Default: true for SELECT."),
                false,
            ),
        ],
    },
    Tool {
        name: "dependents",
        title: "Statements standing on a statement",
        description: "List the statements that a retraction of eid would cascade to (its layers and \
memberships), on a view. Use it to preview a correction.",
        writes: false,
        idempotent: true,
        args: &[("eid", Schema::Eid, true), ("view", Schema::View, false)],
    },
    Tool {
        name: "export_bundle",
        title: "Export a fact bundle",
        description: "Export the statement eid with its layers and evidence as a tiramemsu-bundle/1 JSON \
document, on a view. import_bundle reads it back into this or another memory.",
        writes: false,
        idempotent: true,
        args: &[("eid", Schema::Eid, true), ("view", Schema::View, false)],
    },
    Tool {
        name: "import_bundle",
        title: "Import a fact bundle",
        description: "Import a tiramemsu-bundle/1 JSON document in one transaction. Statements already \
live are reused (new: false). A malformed bundle is refused and nothing is committed.",
        writes: true,
        idempotent: true,
        args: &[(
            "bundle",
            Schema::Object("A tiramemsu-bundle/1 document, as export_bundle returns it."),
            true,
        )],
    },
    Tool {
        name: "text_search",
        title: "Search memory by text",
        description: "Find statements whose string object matches the words, ranked by lexical score, \
then confidence, confirmations, authors and recency; each hit carries its evidence. Needs the text \
index (start the server with --text-index).",
        writes: false,
        idempotent: true,
        args: &[
            ("text", Schema::Text("The words to find; a trailing * is a prefix."), true),
            (
                "mode",
                Schema::Enum("How the words combine (default all).", &["all", "any", "phrase"]),
                false,
            ),
            ("graphs", Schema::Terms("Keep statements in one of these graphs."), false),
            ("predicates", Schema::Terms("Keep statements with one of these predicates."), false),
            ("limit", Schema::Count("The most hits to return."), false),
            (
                "confidence",
                Schema::Term,
                false,
            ),
            ("view", Schema::View, false),
        ],
    },
];

fn tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// Whether `name` is a tool that writes.
pub fn is_write_tool(name: &str) -> bool {
    tool(name).is_some_and(|t| t.writes)
}

/// The MCP definitions of the tools the configuration allows: every tool, or the
/// read tools only in read-only mode.
pub fn definitions(config: &Config) -> Vec<J> {
    TOOLS
        .iter()
        .filter(|t| !(config.read_only && t.writes))
        .map(|t| {
            let mut props = Map::new();
            let mut required = Vec::new();
            for (name, schema, req) in t.args {
                props.insert((*name).into(), schema.json());
                if *req {
                    required.push(*name);
                }
            }
            json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": {
                    "type": "object",
                    "properties": props,
                    "required": required,
                    "additionalProperties": false,
                },
                "annotations": {
                    "title": t.title,
                    "readOnlyHint": !t.writes,
                    "destructiveHint": false,
                    "idempotentHint": t.idempotent,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

/// Checks the argument object of `t`: an object (or nothing), no path-like key,
/// and no key the tool does not declare.
fn check_args<'a>(t: &Tool, args: &'a J) -> Result<&'a Map<String, J>, ToolError> {
    static EMPTY: std::sync::OnceLock<Map<String, J>> = std::sync::OnceLock::new();
    let o = match args {
        J::Null => EMPTY.get_or_init(Map::new),
        J::Object(o) => o,
        _ => return Err(ToolError::arg("tool arguments must be an object")),
    };
    for k in o.keys() {
        if PATH_KEYS.contains(&k.as_str()) {
            return Err(ToolError::new(
                "PathNotAllowed",
                format!(
                    "argument {k:?} would select a database; this server only serves the file it was started with"
                ),
            ));
        }
        if !t.args.iter().any(|(name, _, _)| name == k) {
            return Err(ToolError::arg(format!(
                "{} takes no argument {k:?}",
                t.name
            )));
        }
    }
    for (name, _, req) in t.args {
        if *req && o.get(*name).is_none_or(J::is_null) {
            return Err(ToolError::arg(format!("`{name}` is required")));
        }
    }
    Ok(o)
}

/// Runs the tool `name` with `args` on `db` under `config`.
///
/// # Errors
///
/// `ReadOnly` for a write tool in read-only mode, `PathNotAllowed` for an
/// argument naming a file, `InvalidArgument` for a malformed call, and the
/// bridge's codes for what the database refuses. `None` for an unknown tool.
pub fn call(db: &Database, config: &Config, name: &str, args: &J) -> Option<Result<J, ToolError>> {
    let t = tool(name)?;
    Some(run(db, config, t, args))
}

fn run(db: &Database, config: &Config, t: &Tool, args: &J) -> Result<J, ToolError> {
    // checked before the arguments, so nothing of a write call is even parsed
    if t.writes && config.read_only {
        return Err(ToolError::new(
            "ReadOnly",
            format!("{} writes, and this server is read-only", t.name),
        ));
    }
    let a = check_args(t, args)?;
    let budget = config.budget.to_json();
    let get = |k: &str| a.get(k).filter(|v| !v.is_null()).cloned();
    let view = || -> J { get("view").unwrap_or_else(|| json!({ "kind": "now" })) };
    let read = |op: &str, mut call: Map<String, J>| -> Result<(J, J), ToolError> {
        let v = view();
        call.insert("view".into(), v.clone());
        call.insert("budget".into(), budget.clone());
        Ok((db.call(op, &J::Object(call))?, v))
    };
    let transact = |ops: J| -> Result<J, ToolError> {
        Ok(db.call("transact", &json!({ "ops": ops, "budget": budget }))?)
    };
    match t.name {
        "assert" => {
            let mut op = Map::new();
            op.insert("op".into(), json!("assert"));
            op.insert("as".into(), json!("fact"));
            for k in ["s", "p", "o", "validFrom", "validTo", "onExisting"] {
                if let Some(v) = get(k) {
                    op.insert(k.into(), v);
                }
            }
            let mut ops = vec![J::Object(op)];
            if let Some(g) = get("graph") {
                ops.push(json!({ "op": "addToGraph", "eid": { "ref": "fact" }, "graph": g }));
            }
            let r = transact(J::Array(ops))?;
            let fact = &r["results"][0];
            let mut out = json!({
                "eid": fact["eid"],
                "new": fact["new"],
                "t": r["t"],
            });
            if let Some(m) = r["results"].get(1) {
                out["membership"] = json!({ "eid": m["eid"], "new": m["new"] });
            }
            Ok(out)
        }
        "confirm" => {
            let eid = get("eid").unwrap_or(J::Null);
            let r = transact(json!([{ "op": "confirm", "eid": eid }]))?;
            Ok(json!({ "eid": eid, "confirmation": r["results"][0]["eid"], "t": r["t"] }))
        }
        "supersede" => {
            let eid = get("eid").unwrap_or(J::Null);
            let patch = get("patch").unwrap_or(J::Null);
            let r = transact(json!([{ "op": "supersede", "eid": eid, "patch": patch }]))?;
            Ok(json!({
                "old": eid,
                "eid": r["results"][0]["eid"],
                "t": r["t"],
                "retracted": r["retracted"],
            }))
        }
        "import_bundle" => {
            let bundle = get("bundle").unwrap_or(J::Null);
            // a malformed bundle is the caller's argument error, refused before
            // the transaction opens
            Bundle::from_json(&bundle)
                .map_err(|e| ToolError::arg(format!("malformed bundle: {e}")))?;
            let r = transact(json!([{ "op": "importBundle", "bundle": bundle }]))?;
            let out = &r["results"][0];
            Ok(json!({ "root": out["root"], "statements": out["statements"], "t": r["t"] }))
        }
        "query" => query(&get, read),
        "dependents" => {
            let eid = get("eid").unwrap_or(J::Null);
            let mut c = Map::new();
            c.insert("eid".into(), eid.clone());
            let (r, v) = read("dependents", c)?;
            Ok(json!({ "eid": eid, "view": v, "dependents": r }))
        }
        "export_bundle" => {
            let mut c = Map::new();
            c.insert("eid".into(), get("eid").unwrap_or(J::Null));
            let (r, v) = read("bundle", c)?;
            Ok(json!({ "view": v, "bundle": r }))
        }
        "text_search" => {
            let mut c = Map::new();
            for k in [
                "text",
                "mode",
                "graphs",
                "predicates",
                "limit",
                "confidence",
            ] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            let (r, v) = read("textSearch", c)?;
            Ok(json!({ "view": v, "hits": r }))
        }
        other => Err(ToolError::arg(format!("unknown tool {other:?}"))),
    }
}

/// The `query` tool: SPARQL with query-only text and provenance by default for a
/// `SELECT`, or read-only Cypher; the response names the view and the
/// provenance coverage.
fn query(
    get: &dyn Fn(&str) -> Option<J>,
    read: impl Fn(&str, Map<String, J>) -> Result<(J, J), ToolError>,
) -> Result<J, ToolError> {
    let language = get("language").unwrap_or(J::Null);
    let text = get("text")
        .and_then(|t| t.as_str().map(str::to_string))
        .ok_or_else(|| ToolError::arg("`text` must be a string"))?;
    let provenance = match get("provenance") {
        None => None,
        Some(J::Bool(b)) => Some(b),
        Some(_) => return Err(ToolError::arg("`provenance` must be a boolean")),
    };
    let mut c = Map::new();
    c.insert("text".into(), json!(text));
    match language.as_str() {
        Some("sparql") => {
            if get("params").is_some() {
                return Err(ToolError::arg(
                    "SPARQL takes no `params`; write the values into the text",
                ));
            }
            let cite = provenance.unwrap_or_else(|| sparql_form(&text) == Some(Form::Select));
            c.insert("queryOnly".into(), json!(true));
            c.insert("provenance".into(), json!(cite));
            let (mut r, v) = read("sparql", c)?;
            let coverage = match r.as_object_mut().and_then(|o| o.remove("provenanceGaps")) {
                Some(J::Array(gaps)) if gaps.is_empty() => {
                    json!({ "coverage": "complete", "gaps": [] })
                }
                Some(J::Array(gaps)) => json!({
                    "coverage": "incomplete",
                    "gaps": gaps,
                    "reason": "the query has a recursive path, whose statements are not cited",
                }),
                _ if cite => {
                    json!({ "coverage": "unavailable", "reason": "the result has no rows to cite" })
                }
                _ => json!({
                    "coverage": "unavailable",
                    "reason": "provenance was not requested, or this query form carries none",
                }),
            };
            Ok(json!({ "language": "sparql", "view": v, "result": r, "provenance": coverage }))
        }
        Some("cypher") => {
            if provenance == Some(true) {
                return Err(ToolError::arg(
                    "provenance is a SPARQL option; in Cypher, return relationship variables, which are statement ids",
                ));
            }
            if let Some(p) = get("params") {
                c.insert("params".into(), p);
            }
            let (r, v) = read("cypher", c)?;
            Ok(json!({
                "language": "cypher",
                "view": v,
                "result": r,
                "provenance": {
                    "coverage": "unavailable",
                    "reason": "Cypher results carry no provenance; relationship variables are statement ids",
                },
            }))
        }
        _ => Err(ToolError::arg(
            "`language` must be \"sparql\" or \"cypher\" (raw SQL is not available)",
        )),
    }
}

/// A SPARQL query form, as far as the provenance default needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Select,
    Other,
}

/// The form of SPARQL `text`: the first keyword after the prologue (`BASE`,
/// `PREFIX` and comments). This only picks the provenance default; the engine
/// still parses the text, and query-only mode refuses updates on its own.
fn sparql_form(text: &str) -> Option<Form> {
    let mut rest = text;
    loop {
        rest = rest.trim_start();
        if let Some(r) = rest.strip_prefix('#') {
            rest = r.split_once('\n').map_or("", |(_, r)| r);
            continue;
        }
        let word: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        match word.to_ascii_uppercase().as_str() {
            "BASE" => {
                // BASE <iri>
                let r = &rest[word.len()..];
                rest = r.split_once('>').map(|(_, r)| r)?;
            }
            "PREFIX" => {
                // PREFIX name: <iri>
                let r = &rest[word.len()..];
                rest = r.split_once('>').map(|(_, r)| r)?;
            }
            "SELECT" => return Some(Form::Select),
            "" => return None,
            _ => return Some(Form::Other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sparql_form_skips_the_prologue() {
        assert_eq!(sparql_form("SELECT * {}"), Some(Form::Select));
        assert_eq!(
            sparql_form("# who\nPREFIX ex: <http://x/#a>\n BASE <http://b/> select ?x {}"),
            Some(Form::Select)
        );
        assert_eq!(sparql_form("ASK {}"), Some(Form::Other));
        assert_eq!(sparql_form("INSERT DATA {}"), Some(Form::Other));
        assert_eq!(sparql_form("  "), None);
        assert_eq!(sparql_form("PREFIX ex: <unterminated"), None);
    }

    #[test]
    fn definitions_declare_strict_schemas() {
        let config = Config::new("m.db");
        let all = definitions(&config);
        assert_eq!(all.len(), TOOLS.len());
        for d in &all {
            assert_eq!(d["inputSchema"]["additionalProperties"], json!(false));
            let props = d["inputSchema"]["properties"].as_object().unwrap();
            assert!(PATH_KEYS.iter().all(|k| !props.contains_key(*k)), "{d}");
        }
        let ro = definitions(&Config {
            read_only: true,
            ..config
        });
        let names: Vec<_> = ro.iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(
            names,
            ["query", "dependents", "export_bundle", "text_search"]
        );
        assert!(ro
            .iter()
            .all(|d| d["annotations"]["readOnlyHint"] == json!(true)));
    }
}
