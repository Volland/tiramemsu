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
    /// The bridge error's other fields, such as `program`, `constraint` and
    /// `violations` of a `ConstraintViolated` refusal; empty for most errors.
    pub details: Map<String, J>,
}

impl ToolError {
    /// An error with `code` and `message`.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> ToolError {
        ToolError {
            code: code.into(),
            message: message.into(),
            details: Map::new(),
        }
    }

    fn arg(message: impl Into<String>) -> ToolError {
        ToolError::new("InvalidArgument", message)
    }

    /// The error as `{"code", "message", ...}`, the form of the bridge's errors.
    pub fn to_json(&self) -> J {
        let mut m = self.details.clone();
        m.insert("code".into(), json!(self.code));
        m.insert("message".into(), json!(self.message));
        J::Object(m)
    }
}

impl From<BindError> for ToolError {
    fn from(e: BindError) -> ToolError {
        let mut err = ToolError::new(e.code(), e.to_string());
        if let J::Object(mut m) = e.to_json() {
            m.remove("code");
            m.remove("message");
            err.details = m;
        }
        err
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
    Eids(&'static str),
    Time,
    View,
    Text(&'static str),
    Enum(&'static str, &'static [&'static str]),
    Object(&'static str),
    Terms(&'static str),
    Count(&'static str),
    Flag(&'static str),
    Vector(&'static str),
    #[cfg(any(feature = "n3", feature = "datalog", feature = "elpi"))]
    Iri(&'static str),
    #[cfg(feature = "datalog")]
    Names(&'static str),
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
            Schema::Eids(d) => json!({
                "type": "array",
                "items": {
                    "type": ["integer", "object"],
                    "description": "A statement id: a number or {\"stmt\": n}.",
                },
                "minItems": 1,
                "description": d,
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
            Schema::Vector(d) => json!({
                "type": "array",
                "items": { "type": "number" },
                "description": d,
            }),
            #[cfg(any(feature = "n3", feature = "datalog", feature = "elpi"))]
            Schema::Iri(d) => json!({ "type": ["string", "object"], "description": d }),
            #[cfg(feature = "datalog")]
            Schema::Names(d) => {
                json!({ "type": "array", "items": { "type": "string" }, "description": d })
            }
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
        name: "derive",
        title: "Derive a belief",
        description: "Assert the belief (s, p, o) as concluded from the live statements in support: \
writes the statement and one sys:derivedFrom link per support, in one transaction. When any support is \
later retracted or corrected, the belief is retracted with it, so a summary never outlives its evidence. \
Deriving the same content from the same supports again returns the existing statement (new: false); an \
identical fact asserted independently stays independent. Returns the eid, new and the transaction (t).",
        writes: true,
        idempotent: true,
        args: &[
            ("s", Schema::Term, true),
            ("p", Schema::Term, true),
            ("o", Schema::Term, true),
            (
                "support",
                Schema::Eids("The statements the belief is concluded from (at least one, all live)."),
                true,
            ),
            ("validFrom", Schema::Time, false),
            ("validTo", Schema::Time, false),
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
        name: "conflicts",
        title: "Find conflicting memories",
        description: "List subject/predicate pairs that hold different objects at the same valid time, on a \
view, with the windows where they overlap and the evidence behind every value: statement ids, valid time, \
the asserting transaction, confidence when stated (null otherwise), confirmations, authors and sources. \
Statements with the same object support one value. Nothing is scored or resolved and nothing is written; \
choose explicitly with supersede, confirm or assert. Not available on the history view.",
        writes: false,
        idempotent: true,
        args: &[
            ("s", Schema::Term, false),
            ("p", Schema::Term, false),
            ("limit", Schema::Count("The most conflicts to return."), false),
            ("confidence", Schema::Term, false),
            ("source", Schema::Term, false),
            ("view", Schema::View, false),
        ],
    },
    Tool {
        name: "preview_bundle",
        title: "Preview a bundle import",
        description: "Run a tiramemsu-bundle/1 import as a dry run and report what it would do without \
committing: the proposed (new: true) and reused statements, dependency changes (retractions by \
cardinality-one replacement, memberships), the schema failure if any, the ids the dry run burned, and the \
transaction it was computed against (scope.basis). Graph, history and event log stay unchanged. A preview \
reserves nothing: import_bundle validates again when it applies.",
        writes: false,
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
    Tool {
        name: "define_embedding_model",
        title: "Define an embedding model",
        description: "Declare the embedding model an agent will store vectors for: its IRI, number of \
dimensions, metric (cosine, dot or l2; default cosine) and storage encoding (f32 or f16; default f32). \
Defining the same model again is a no-op (new: false); a different definition for the same IRI is \
refused: use a new IRI. The store never computes embeddings itself.",
        writes: true,
        idempotent: true,
        args: &[
            ("model", Schema::Text("The model IRI, e.g. urn:example:models:minilm-l6."), true),
            ("dimensions", Schema::Count("The number of components of every vector."), true),
            (
                "metric",
                Schema::Enum("How vectors are compared (default cosine).", &["cosine", "dot", "l2"]),
                false,
            ),
            (
                "encoding",
                Schema::Enum("How vectors are stored (default f32).", &["f32", "f16"]),
                false,
            ),
        ],
    },
    Tool {
        name: "set_embedding",
        title: "Store an embedding",
        description: "Store the embedding of a live statement under a defined model, computed by you \
from the statement's text. Storing the identical vector again is a no-op (new: false); a different \
vector for the same statement and model is refused (EmbeddingConflict). A corrected (superseded) fact \
has a new eid and needs its own embedding.",
        writes: true,
        idempotent: true,
        args: &[
            ("eid", Schema::Eid, true),
            ("model", Schema::Text("The model IRI."), true),
            ("vector", Schema::Vector("The embedding, in the model's dimensions."), true),
        ],
    },
    Tool {
        name: "vector_search",
        title: "Search memory by meaning",
        description: "Find the statements whose embedding is closest to a query vector, in the chosen \
view (now, as of a past transaction or instant, history, valid at a time). The search is exact over the \
statements the view selects, so time travel never loses a hit; hits carry similarity and evidence \
(confidence, confirmations, authors, when added).",
        writes: false,
        idempotent: true,
        args: &[
            ("model", Schema::Text("The model IRI."), true),
            ("vector", Schema::Vector("The query embedding, computed with the same model."), true),
            ("graphs", Schema::Terms("Keep statements in one of these graphs."), false),
            ("predicates", Schema::Terms("Keep statements with one of these predicates."), false),
            ("limit", Schema::Count("The most hits to return (default 10)."), false),
            ("confidence", Schema::Term, false),
            (
                "prefilter",
                Schema::Count(
                    "Score only this many candidates chosen by sign-bit distance, then re-rank \
them exactly (faster, not exhaustive).",
                ),
                false,
            ),
            ("view", Schema::View, false),
        ],
    },
    Tool {
        name: "recall",
        title: "Recall memory by words and meaning",
        description: "Hybrid recall: text search and vector search in the chosen view, fused by \
reciprocal rank, optionally expanded to each hit's layers (source, confidence, ...) and to other facts \
about its subject. Give text, a model with a vector, or both. Text needs the text index (--text-index).",
        writes: false,
        idempotent: true,
        args: &[
            ("text", Schema::Text("The words to find."), false),
            (
                "mode",
                Schema::Enum("How the words combine (default all).", &["all", "any", "phrase"]),
                false,
            ),
            ("model", Schema::Text("The model IRI of the vector."), false),
            ("vector", Schema::Vector("The query embedding."), false),
            ("graphs", Schema::Terms("Keep statements in one of these graphs."), false),
            ("predicates", Schema::Terms("Keep statements with one of these predicates."), false),
            ("limit", Schema::Count("The most fused hits (default 10)."), false),
            (
                "depth",
                Schema::Count("Candidates taken from each channel (default max(4 × limit, 50))."),
                false,
            ),
            (
                "expand",
                Schema::Count("Per hit, add up to this many layers and subject facts (default 0)."),
                false,
            ),
            ("view", Schema::View, false),
        ],
    },
    Tool {
        name: "unembedded",
        title: "List statements without an embedding",
        description: "List the statement ids in the chosen view that have no embedding under a \
model yet: the backlog to embed (compute each vector from the statement, then call set_embedding).",
        writes: false,
        idempotent: true,
        args: &[
            ("model", Schema::Text("The model IRI."), true),
            ("limit", Schema::Count("The most ids to return."), false),
            ("view", Schema::View, false),
        ],
    },
    Tool {
        name: "save_answer",
        title: "Save an answer",
        description: "Run a read-only SPARQL (SELECT, ASK) or openCypher query on a view and save its \
answer under a name, with the statements it cited. Later writes mark it: stale when a cited statement is \
retracted or superseded, recheck when anything else could have changed it (check_answers). Saving the \
same name replaces the answer. It writes only the derived answer record, never the facts.",
        writes: true,
        idempotent: true,
        args: &[
            ("name", Schema::Text("The answer's name."), true),
            (
                "language",
                Schema::Enum("The query language.", &["sparql", "cypher"]),
                true,
            ),
            ("text", Schema::Text("The query text."), true),
            ("params", Schema::Object("Cypher parameters ($name)."), false),
            ("view", Schema::View, false),
        ],
    },
    Tool {
        name: "saved_answers",
        title: "Read saved answers",
        description: "Read one saved answer by name, or all of them: the query, view and result as saved, \
the cited statements, the coverage reasons that keep them from proving freshness, the checkpoint, and the \
status (fresh, recheck or stale) with the event that caused it. Run check_answers first for current marks.",
        writes: false,
        idempotent: true,
        args: &[("name", Schema::Text("Only this answer."), false)],
    },
    Tool {
        name: "check_answers",
        title: "Check saved answers",
        description: "Process the events since the last check and return the new invalidations, each once: \
stale (a cited statement was retracted or superseded) or recheck (another write, or the clock, may have \
changed the answer). Fixed historical views stay fresh. Only refresh_answer makes an answer fresh again.",
        writes: true,
        idempotent: true,
        args: &[],
    },
    Tool {
        name: "refresh_answer",
        title: "Refresh a saved answer",
        description: "Re-run a saved answer with its saved query, parameters and view. On success the \
result, citations and checkpoint are replaced and the answer is fresh; on failure it keeps its old result \
and mark, and records the error.",
        writes: true,
        idempotent: false,
        args: &[("name", Schema::Text("The answer's name."), true)],
    },
    #[cfg(feature = "n3")]
    Tool {
        name: "reason",
        title: "Reason with N3 rules",
        description: "Run Notation3 rules over memory on a view and return what follows, without writing \
anything. Give the rules as N3 text (rules) or name a rule set stored with store_rules (ruleSet). A conclusion \
holds at an instant only if it follows from the facts valid at that instant, so facts that were never true \
together never combine; each conclusion carries its valid time and a proof (the rule's index and the \
statements or earlier conclusions it used). Rule text may use the prefixes v: (urn:tiramemsu:v:), tm: and n3: \
without declaring them; a statement is <urn:tiramemsu:stmt:N> with tm:content, tm:validFrom and tm:validTo. \
An asOf view reasons over memory as believed then, with the rules believed then.",
        writes: false,
        idempotent: true,
        args: &[
            ("rules", Schema::Text("N3 rule text (or give ruleSet)."), false),
            ("ruleSet", Schema::Iri(RULE_SET), false),
            ("scope", Schema::Object(SCOPE), false),
            ("window", Schema::Object(WINDOW), false),
            (
                "time",
                Schema::Enum(
                    "pointwise (default): conclusions respect valid time; atemporal: ignore valid time.",
                    &["pointwise", "atemporal"],
                ),
                false,
            ),
            (
                "maxSegments",
                Schema::Count("The most valid-time segments a run reasons over (default 1024); past it the status is incomplete."),
                false,
            ),
            ("proofN3", Schema::Flag("Add the reasoner's N3 proof text."), false),
            ("limits", Schema::Object(LIMITS), false),
            ("view", Schema::View, false),
        ],
    },
    #[cfg(feature = "n3")]
    Tool {
        name: "store_rules",
        title: "Store an N3 rule set",
        description: "Store N3 rule text as the rule set ruleSet: one statement (ruleSet n3:source \"<text>\"). \
Storing new text for an existing rule set supersedes the old version, which stays visible in asOf views; \
storing the same text again changes nothing. Text that does not parse is refused (N3Parse) and nothing is \
written. Returns the rule-set statement (eid), whether anything changed and the transaction (t).",
        writes: true,
        idempotent: true,
        args: &[
            ("ruleSet", Schema::Iri(RULE_SET), true),
            ("n3", Schema::Text("The N3 rule text."), true),
        ],
    },
    #[cfg(feature = "n3")]
    Tool {
        name: "materialize",
        title: "Materialize a rule set",
        description: "Reason with the stored rule set ruleSet on a snapshot and write its conclusions as derived \
beliefs, in one transaction: each conclusion is supported by its premises and by the rule-set statement, so it \
is retracted when a premise is retracted or corrected, or when the rule set is edited. Re-runs reuse existing \
conclusions and widen their valid time. Rule sets with negation, aggregation or the clock are refused \
(N3NotMaterializable), and a fired inference fuse aborts the run (N3Contradiction). dryRun reports what would \
be written and writes nothing.",
        writes: true,
        idempotent: true,
        args: &[
            ("ruleSet", Schema::Iri(RULE_SET), true),
            ("scope", Schema::Object(SCOPE), false),
            ("window", Schema::Object(WINDOW), false),
            (
                "maxSegments",
                Schema::Count("The most valid-time segments the run reasons over (default 1024)."),
                false,
            ),
            ("limits", Schema::Object(LIMITS), false),
            (
                "requireComplete",
                Schema::Flag("Refuse to write when the reasoning run is incomplete."),
                false,
            ),
            ("dryRun", Schema::Flag("Report what would be written and write nothing."), false),
        ],
    },
    #[cfg(feature = "datalog")]
    Tool {
        name: "datalog_query",
        title: "Query memory with Datalog",
        description: "Run a Datalog program over memory on a view and return the rows of a goal, without \
writing anything. Give the program as Datalog text (program) or name a program stored with store_program \
(stored); goal is a body such as \"reports(v:alice, X)\" (default: the program's first ?- goal). Stored facts \
are atoms p(s, o) over predicate IRIs; rules may recurse, negate (stratified), aggregate and use metric \
operators over valid time. A row holds over the valid time in which its binding follows from the facts valid \
then; each row carries those intervals and, with proof, the rule and statements behind each interval. Text \
may use the prefixes v: (urn:tiramemsu:v:), rdf:, rdfs:, xsd:, owl:, dl:, sys: and tm: without declaring \
them. An asOf view runs over memory as believed then, with the program believed then.",
        writes: false,
        idempotent: true,
        args: &[
            ("program", Schema::Text("Datalog program text (or give stored)."), false),
            ("stored", Schema::Iri(PROGRAM), false),
            (
                "goal",
                Schema::Text("The goal: a Datalog body such as reports(v:alice, X), with or without ?-."),
                false,
            ),
            ("scope", Schema::Object(SCOPE), false),
            ("window", Schema::Object(WINDOW), false),
            (
                "time",
                Schema::Enum(
                    "pointwise (default): rows respect valid time; atemporal: ignore valid time.",
                    &["pointwise", "atemporal"],
                ),
                false,
            ),
            ("limits", Schema::Object(DL_LIMITS), false),
            (
                "demand",
                Schema::Flag("Evaluate a goal with constants for demand only (magic sets). Default: true."),
                false,
            ),
            ("proof", Schema::Flag("Add each row's derivation per interval."), false),
            ("access", Schema::Enum(ACCESS, &["auto", "probe", "load"]), false),
            ("parallelThreshold", Schema::Count(PARALLEL), false),
            (
                "requireComplete",
                Schema::Flag("Fail with DatalogIncomplete instead of returning partial rows."),
                false,
            ),
            ("view", Schema::View, false),
        ],
    },
    #[cfg(feature = "datalog")]
    Tool {
        name: "check_constraints",
        title: "Check Datalog constraints",
        description: "Evaluate the constraints of a Datalog program (constraint name warn|error :- body.) over \
memory on a view and return the violated ones, without writing anything. Give the program as text (program) \
or name a stored program (stored); only restricts the check to these constraint names. Each violation carries \
the body's bindings, the statements its derivation used (eids) and the valid time over which it holds, so \
facts that were never true together never violate a constraint. An asOf view checks memory as believed then.",
        writes: false,
        idempotent: true,
        args: &[
            ("program", Schema::Text("Datalog program text (or give stored)."), false),
            ("stored", Schema::Iri(PROGRAM), false),
            ("only", Schema::Names("Check only the constraints with these names."), false),
            ("scope", Schema::Object(SCOPE), false),
            ("window", Schema::Object(WINDOW), false),
            (
                "time",
                Schema::Enum(
                    "pointwise (default): violations respect valid time; atemporal: ignore valid time.",
                    &["pointwise", "atemporal"],
                ),
                false,
            ),
            ("limits", Schema::Object(DL_LIMITS), false),
            ("access", Schema::Enum(ACCESS, &["auto", "probe", "load"]), false),
            ("parallelThreshold", Schema::Count(PARALLEL), false),
            (
                "requireComplete",
                Schema::Flag("Fail with DatalogIncomplete instead of returning a partial check."),
                false,
            ),
            ("view", Schema::View, false),
        ],
    },
    #[cfg(feature = "datalog")]
    Tool {
        name: "store_program",
        title: "Store a Datalog program",
        description: "Store Datalog text as the program program: one statement (program dl:source \"<text>\"). \
Storing new text for an existing program supersedes the old version, which stays visible in asOf views; \
storing the same text again changes nothing. Text that does not parse or is unsafe, unstratifiable or \
unbounded is refused (DatalogParse, DatalogUnsafe, ...), and a node that holds N3 rules is refused \
(RuleLanguageConflict); nothing is written then. Returns the program statement (eid), whether anything \
changed and the transaction (t).",
        writes: true,
        idempotent: true,
        args: &[
            ("program", Schema::Iri(PROGRAM), true),
            ("text", Schema::Text("The Datalog program text."), true),
        ],
    },
    #[cfg(feature = "datalog")]
    Tool {
        name: "enforce_program",
        title: "Enforce a Datalog program's constraints",
        description: "Enforce the error constraints of a stored Datalog program (on, the default) at every \
write, or stop enforcing them (on: false): one statement (program dl:enforced true). Turning it on first \
checks the program's error constraints over all of memory and is refused with ConstraintViolated when memory \
already violates one, or ConstraintNotEnforceable when one cannot be checked from a write's changes. From \
then on any write that would introduce a violation is refused with ConstraintViolated and writes nothing; the \
error lists the violations (bindings, eids, valid time). warn constraints never refuse a write. Returns the \
enforcement statement (eid, null when turned off) and the transaction (t).",
        writes: true,
        idempotent: true,
        args: &[
            ("program", Schema::Iri(PROGRAM), true),
            (
                "on",
                Schema::Flag("true (default) enforces the program; false stops enforcing it."),
                false,
            ),
        ],
    },
    #[cfg(feature = "datalog")]
    Tool {
        name: "materialize_program",
        title: "Materialize a Datalog program",
        description: "Run the stored Datalog program program on a snapshot and write its conclusions as derived \
beliefs, in one transaction: each conclusion is supported by its premises and by the program statement, so it \
is retracted when a premise is retracted or corrected, or when the program is edited. Re-runs reuse existing \
conclusions, widen their valid time and reconcile their support. Conclusions that cannot be stored are \
reported as unstorable, and needsRerunAfterWrites lists the predicates the run's own writes feed. dryRun \
reports what would be written and writes nothing. An unknown program is refused (UnknownProgram).",
        writes: true,
        idempotent: true,
        args: &[
            ("program", Schema::Iri(PROGRAM), true),
            ("scope", Schema::Object(SCOPE), false),
            ("window", Schema::Object(WINDOW), false),
            ("limits", Schema::Object(DL_LIMITS), false),
            (
                "requireComplete",
                Schema::Flag("Refuse to write when the run is incomplete."),
                false,
            ),
            ("dryRun", Schema::Flag("Report what would be written and write nothing."), false),
        ],
    },
    #[cfg(feature = "elpi")]
    Tool {
        name: "elpi_query",
        title: "Query memory with ELPI",
        description: "Solve an ELPI (λProlog) goal over memory on a view and return one row per solution, \
without writing anything. Give the program as ELPI text (program) or name a program stored with \
store_elpi_program (stored); goal is an ELPI query such as reports (iri \"urn:tiramemsu:v:alice\") Y. Programs \
read statements with tm.triple S P O E, valid at one instant (validAt, default now); terms are (iri \"…\"), \
(str \"…\"), (num N), (dbl F), (bool \"true\"), (lang Lex Tag), (lit Lex Datatype) and statements (eid N). A \
program may include stored programs with accumulate <iri>. fuel bounds the resolution steps.",
        writes: false,
        idempotent: true,
        args: &[
            ("program", Schema::Text("ELPI program text (or give stored)."), false),
            ("stored", Schema::Iri(ELPI_PROGRAM), false),
            ("goal", Schema::Text("The ELPI goal, such as reports (iri \"urn:tiramemsu:v:alice\") Y."), true),
            ("validAt", Schema::Text("The valid instant the program reads (epoch ms or a date-time)."), false),
            ("fuel", Schema::Count("Resolution steps (default: the engine's budget)."), false),
            ("limit", Schema::Count("At most this many solutions (default: all)."), false),
            ("view", Schema::View, false),
        ],
    },
    #[cfg(feature = "elpi")]
    Tool {
        name: "elpi_check",
        title: "Check ELPI constraints",
        description: "Run an ELPI program's tm.violation Severity Name Message Eids over memory on a view and \
return every violation (severity error or warn, name, message, the statements it cites and the valid time \
over which they hold together), without writing anything. Give the program as text (program) or name a \
stored program (stored).",
        writes: false,
        idempotent: true,
        args: &[
            ("program", Schema::Text("ELPI program text (or give stored)."), false),
            ("stored", Schema::Iri(ELPI_PROGRAM), false),
            ("fuel", Schema::Count("Resolution steps (default: the engine's budget)."), false),
            ("view", Schema::View, false),
        ],
    },
    #[cfg(feature = "elpi")]
    Tool {
        name: "store_elpi_program",
        title: "Store an ELPI program",
        description: "Store ELPI text as the program program: one statement (program elpi:source \"<text>\"). \
New text supersedes the old version, which stays visible in asOf views; the same text changes nothing. Text \
that does not load is refused (ElpiProgram), an include cycle is refused (ProgramCycle), and a node holding \
N3 rules or a Datalog program is refused (RuleLanguageConflict). Returns the program statement (eid), \
whether anything changed and the transaction (t).",
        writes: true,
        idempotent: true,
        args: &[
            ("program", Schema::Iri(ELPI_PROGRAM), true),
            ("text", Schema::Text("The ELPI program text."), true),
        ],
    },
    #[cfg(feature = "elpi")]
    Tool {
        name: "enforce_elpi",
        title: "Enforce an ELPI program's constraints",
        description: "Enforce a stored ELPI program (on, the default) at every write, or stop (on: false): one \
statement (program elpi:enforced true). Turning it on runs the program's full tm.violation check and is \
refused with ConstraintViolated when memory already violates an error constraint. From then on every write \
runs tm.violation-delta, seeded by the write's added and retracted statements, and a violation refuses the \
write. Returns the enforcement statement (eid, null when turned off) and the transaction (t).",
        writes: true,
        idempotent: true,
        args: &[
            ("program", Schema::Iri(ELPI_PROGRAM), true),
            ("on", Schema::Flag("true (default) enforces the program; false stops enforcing it."), false),
        ],
    },
    #[cfg(feature = "elpi")]
    Tool {
        name: "materialize_elpi",
        title: "Materialize an ELPI program",
        description: "Run the stored ELPI program's tm.conclude Fact Support on a snapshot and write each \
conclusion as a derived belief supported by the statements it lists, the program statement and the programs \
it includes, in one transaction; earlier conclusions the run no longer derives are retracted. dryRun reports \
what would be written and writes nothing.",
        writes: true,
        idempotent: true,
        args: &[
            ("program", Schema::Iri(ELPI_PROGRAM), true),
            ("fuel", Schema::Count("Resolution steps (default: the engine's budget)."), false),
            ("dryRun", Schema::Flag("Report what would be written and write nothing."), false),
        ],
    },
];

#[cfg(feature = "elpi")]
const ELPI_PROGRAM: &str =
    "An ELPI program stored with store_elpi_program: an IRI as a plain string, or a term object.";

#[cfg(feature = "n3")]
const RULE_SET: &str = "A rule set: an IRI as a plain string, or a term object.";
#[cfg(any(feature = "n3", feature = "datalog"))]
const SCOPE: &str = "The statements reasoned over: \"all\" (default), {\"graphs\": [terms]} (members of these \
named graphs) or {\"seeds\": {\"nodes\": [terms], \"hops\": n}} (statements within hops of the seed nodes).";
#[cfg(any(feature = "n3", feature = "datalog"))]
const WINDOW: &str =
    "The valid-time window of the run: {\"from\": time, \"to\": time}, either bound optional. \
Default: unbounded.";
#[cfg(feature = "n3")]
const LIMITS: &str =
    "The reasoner's limits per run: {\"maxIterations\", \"maxMatchSteps\", \"maxBackwardDepth\", \
\"maxTermBytes\"}.";
#[cfg(feature = "datalog")]
const PROGRAM: &str =
    "A Datalog program stored with store_program: an IRI as a plain string, or a term object.";
#[cfg(feature = "datalog")]
const DL_LIMITS: &str =
    "The engine's limits per run: {\"maxIterations\", \"maxFacts\", \"maxFragments\", \
\"maxNewValues\"}; past one the status is incomplete.";
#[cfg(feature = "datalog")]
const ACCESS: &str =
    "How stored atoms are read: auto (default), probe (index lookups) or load (scan once).";
#[cfg(feature = "datalog")]
const PARALLEL: &str = "Deltas larger than this run in parallel (default 4096; 0: never).";

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
        // @tg: implements:: [[openspec:derived-beliefs#Derive on every surface]] {since: "2026-10-28"}
        "derive" => {
            let mut op = Map::new();
            op.insert("op".into(), json!("derive"));
            for k in ["s", "p", "o", "support", "validFrom", "validTo"] {
                if let Some(v) = get(k) {
                    op.insert(k.into(), v);
                }
            }
            let r = transact(J::Array(vec![J::Object(op)]))?;
            let d = &r["results"][0];
            Ok(json!({ "eid": d["eid"], "new": d["new"], "t": r["t"] }))
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
        "conflicts" => {
            let mut c = Map::new();
            for k in ["s", "p", "limit", "confidence", "source"] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            let (r, v) = read("conflicts", c)?;
            Ok(json!({ "view": v, "conflicts": r }))
        }
        "preview_bundle" => {
            let bundle = get("bundle").unwrap_or(J::Null);
            Bundle::from_json(&bundle)
                .map_err(|e| ToolError::arg(format!("malformed bundle: {e}")))?;
            let p = db.call(
                "previewBundle",
                &json!({ "bundle": bundle, "budget": budget }),
            )?;
            Ok(json!({ "preview": p }))
        }
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
        "define_embedding_model" => {
            let mut op = Map::new();
            op.insert("op".into(), json!("defineEmbeddingModel"));
            for k in ["model", "dimensions", "metric", "encoding"] {
                if let Some(v) = get(k) {
                    op.insert(k.into(), v);
                }
            }
            let r = transact(json!([J::Object(op)]))?;
            Ok(json!({ "model": get("model"), "new": r["results"][0]["new"], "t": r["t"] }))
        }
        "set_embedding" => {
            let mut op = Map::new();
            op.insert("op".into(), json!("setEmbedding"));
            for k in ["eid", "model", "vector"] {
                if let Some(v) = get(k) {
                    op.insert(k.into(), v);
                }
            }
            let r = transact(json!([J::Object(op)]))?;
            Ok(json!({ "eid": get("eid"), "new": r["results"][0]["new"], "t": r["t"] }))
        }
        "vector_search" => {
            let mut c = Map::new();
            for k in [
                "model",
                "vector",
                "graphs",
                "predicates",
                "limit",
                "prefilter",
                "confidence",
            ] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            let (r, v) = read("vectorSearch", c)?;
            Ok(
                json!({ "view": v, "hits": r["hits"], "candidates": r["candidates"], "exact": r["exact"] }),
            )
        }
        "recall" => {
            let mut c = Map::new();
            for k in [
                "text",
                "mode",
                "model",
                "vector",
                "graphs",
                "predicates",
                "limit",
                "depth",
                "expand",
            ] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            let (r, v) = read("recall", c)?;
            Ok(json!({ "view": v, "hits": r }))
        }
        "unembedded" => {
            let mut c = Map::new();
            for k in ["model", "limit"] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            let (r, v) = read("unembedded", c)?;
            Ok(json!({ "view": v, "eids": r }))
        }
        "save_answer" => {
            let mut c = Map::new();
            for k in ["name", "language", "text", "params", "view"] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            c.insert("budget".into(), budget.clone());
            Ok(json!({ "answer": db.call("saveAnswer", &J::Object(c))? }))
        }
        "saved_answers" => match get("name") {
            Some(name) => {
                let a = db.call("savedAnswer", &json!({ "name": name }))?;
                Ok(json!({ "answers": if a.is_null() { json!([]) } else { json!([a]) } }))
            }
            None => Ok(json!({ "answers": db.call("savedAnswers", &J::Null)? })),
        },
        "check_answers" => Ok(json!({ "invalidations": db.call("checkSavedAnswers", &J::Null)? })),
        "refresh_answer" => {
            let name = get("name").unwrap_or(J::Null);
            let a = db.call("refreshAnswer", &json!({ "name": name, "budget": budget }))?;
            Ok(json!({ "answer": a }))
        }
        // @tg: implements:: [[openspec:n3-reasoning#Reasoning surfaces]] {since: "2026-10-28"}
        #[cfg(feature = "n3")]
        "reason" => {
            let mut c = Map::new();
            for k in [
                "rules",
                "ruleSet",
                "scope",
                "window",
                "time",
                "maxSegments",
                "proofN3",
                "limits",
            ] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            let (mut r, v) = read("reason", c)?;
            r["view"] = v;
            Ok(r)
        }
        // @tg: implements:: [[openspec:n3-rule-sets#Rule-set surfaces]] {since: "2026-10-28"}
        #[cfg(feature = "n3")]
        "store_rules" => {
            let op = json!({ "op": "storeRules", "ruleSet": get("ruleSet"), "n3": get("n3") });
            let r = transact(json!([op]))?;
            let changed = r["asserted"].as_array().is_some_and(|a| !a.is_empty());
            Ok(json!({
                "eid": r["results"][0]["eid"],
                "changed": changed,
                "superseded": r["superseded"],
                "t": r["t"],
            }))
        }
        // @tg: implements:: [[openspec:n3-materialization#Materialization surfaces]] {since: "2026-10-28"}
        #[cfg(feature = "n3")]
        "materialize" => {
            let mut c = Map::new();
            for k in [
                "ruleSet",
                "scope",
                "window",
                "maxSegments",
                "limits",
                "requireComplete",
                "dryRun",
            ] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            c.insert("budget".into(), budget.clone());
            Ok(db.call("materialize", &J::Object(c))?)
        }
        // @tg: implements:: [[openspec:datalog-programs#Datalog read surfaces]] {since: "2026-10-28"}
        #[cfg(feature = "datalog")]
        "datalog_query" => {
            let mut c = Map::new();
            for k in [
                "program",
                "stored",
                "goal",
                "scope",
                "window",
                "time",
                "limits",
                "demand",
                "proof",
                "access",
                "parallelThreshold",
                "requireComplete",
            ] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            let (mut r, v) = read("datalog", c)?;
            r["view"] = v;
            Ok(r)
        }
        // @tg: implements:: [[openspec:datalog-constraints#Check constraints]] {since: "2026-10-28"}
        #[cfg(feature = "datalog")]
        "check_constraints" => {
            let mut c = Map::new();
            for k in [
                "program",
                "stored",
                "only",
                "scope",
                "window",
                "time",
                "limits",
                "access",
                "parallelThreshold",
                "requireComplete",
            ] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            let (mut r, v) = read("check", c)?;
            r["view"] = v;
            Ok(r)
        }
        // @tg: implements:: [[openspec:datalog-programs#Store a program]] {since: "2026-10-28"}
        #[cfg(feature = "datalog")]
        "store_program" => {
            let op =
                json!({ "op": "storeProgram", "program": get("program"), "text": get("text") });
            let r = transact(json!([op]))?;
            let changed = r["asserted"].as_array().is_some_and(|a| !a.is_empty());
            Ok(json!({
                "eid": r["results"][0]["eid"],
                "changed": changed,
                "superseded": r["superseded"],
                "t": r["t"],
            }))
        }
        // @tg: implements:: [[openspec:datalog-constraints#Constraint surfaces]] {since: "2026-10-28"}
        #[cfg(feature = "datalog")]
        "enforce_program" => {
            let on = get("on").unwrap_or(J::Bool(true));
            let op = json!({ "op": "enforce", "program": get("program"), "on": on });
            let r = transact(json!([op]))?;
            Ok(json!({ "eid": r["results"][0]["eid"], "on": on, "t": r["t"] }))
        }
        // @tg: implements:: [[openspec:datalog-materialization#Materialization surfaces]]
        #[cfg(feature = "datalog")]
        "materialize_program" => {
            let mut c = Map::new();
            c.insert("stored".into(), get("program").unwrap_or(J::Null));
            for k in ["scope", "window", "limits", "requireComplete", "dryRun"] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            c.insert("budget".into(), budget.clone());
            Ok(db.call("materializeProgram", &J::Object(c))?)
        }
        // @tg: implements:: [[elpi#Own Calls]] {since: "2026-10-31"}
        // @tg: implements:: [[openspec:elpi-query-calls#ELPI bridge operations]] {since: "2026-10-31"}
        #[cfg(feature = "elpi")]
        "elpi_query" | "elpi_check" => {
            let mut c = Map::new();
            for k in ["program", "stored", "goal", "validAt", "fuel", "limit"] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            let op = if t.name == "elpi_query" {
                "elpi"
            } else {
                "elpiCheck"
            };
            let (mut r, v) = read(op, c)?;
            r["view"] = v;
            Ok(r)
        }
        #[cfg(feature = "elpi")]
        "store_elpi_program" => {
            let op =
                json!({ "op": "storeElpiProgram", "program": get("program"), "text": get("text") });
            let r = transact(json!([op]))?;
            let changed = r["asserted"].as_array().is_some_and(|a| !a.is_empty());
            Ok(json!({
                "eid": r["results"][0]["eid"],
                "changed": changed,
                "superseded": r["superseded"],
                "t": r["t"],
            }))
        }
        #[cfg(feature = "elpi")]
        "enforce_elpi" => {
            let on = get("on").unwrap_or(J::Bool(true));
            let op = json!({ "op": "enforceElpi", "program": get("program"), "on": on });
            let r = transact(json!([op]))?;
            Ok(json!({ "eid": r["results"][0]["eid"], "on": on, "t": r["t"] }))
        }
        #[cfg(feature = "elpi")]
        "materialize_elpi" => {
            let mut c = Map::new();
            c.insert("stored".into(), get("program").unwrap_or(J::Null));
            for k in ["fuel", "dryRun"] {
                if let Some(v) = get(k) {
                    c.insert(k.into(), v);
                }
            }
            c.insert("budget".into(), budget.clone());
            Ok(db.call("materializeElpi", &J::Object(c))?)
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
        let mut expected = vec![
            "query",
            "dependents",
            "export_bundle",
            "conflicts",
            "preview_bundle",
            "text_search",
            "vector_search",
            "recall",
            "unembedded",
            "saved_answers",
        ];
        if cfg!(feature = "n3") {
            expected.push("reason");
        }
        if cfg!(feature = "datalog") {
            expected.extend(["datalog_query", "check_constraints"]);
        }
        if cfg!(feature = "elpi") {
            expected.extend(["elpi_query", "elpi_check"]);
        }
        assert_eq!(names, expected);
        assert!(ro
            .iter()
            .all(|d| d["annotations"]["readOnlyHint"] == json!(true)));
    }
}
