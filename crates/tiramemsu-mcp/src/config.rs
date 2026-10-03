//! The server configuration, from command-line arguments.

use std::path::PathBuf;

use serde_json::{json, Map, Value as J};

/// The default deadline of one tool call, in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// The default cap on the rows one tool call may decode.
pub const DEFAULT_MAX_ROWS: u64 = 10_000;
/// The default cap on the decoded bytes of one tool call (8 MiB).
pub const DEFAULT_MAX_BYTES: u64 = 8 << 20;

/// What the server opens and allows. Tool requests cannot change any of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// The database file: the only file the server opens.
    pub db: PathBuf,
    /// Refuse every write tool before a transaction starts, and leave the write
    /// tools out of `tools/list`.
    pub read_only: bool,
    /// The bounds applied to every tool call.
    pub budget: Budget,
    /// Build the derived text index at open, so `text_search` works on a file
    /// that has never had it.
    pub text_index: bool,
}

impl Config {
    /// A configuration for `db` with the default budget, read-write and without
    /// building the text index.
    pub fn new(db: impl Into<PathBuf>) -> Config {
        Config {
            db: db.into(),
            read_only: false,
            budget: Budget::default(),
            text_index: false,
        }
    }
}

/// The bounds of one tool call, mapped to a `tiramemsu::QueryBudget`. `None` is
/// unbounded; on the command line `0` means `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// The longest one call may take, in milliseconds.
    pub timeout_ms: Option<u64>,
    /// The longest one call may wait for a read connection, in milliseconds.
    pub reader_timeout_ms: Option<u64>,
    /// The most rows one call may decode.
    pub max_rows: Option<u64>,
    /// The most decoded bytes one call may produce.
    pub max_bytes: Option<u64>,
}

impl Default for Budget {
    fn default() -> Budget {
        Budget {
            timeout_ms: Some(DEFAULT_TIMEOUT_MS),
            reader_timeout_ms: None,
            max_rows: Some(DEFAULT_MAX_ROWS),
            max_bytes: Some(DEFAULT_MAX_BYTES),
        }
    }
}

impl Budget {
    /// The JSON bridge's budget object (`timeoutMs`, `readerTimeoutMs`, `maxRows`,
    /// `maxBytes`), or `null` when nothing is bounded.
    pub fn to_json(&self) -> J {
        let mut o = Map::new();
        for (k, v) in [
            ("timeoutMs", self.timeout_ms),
            ("readerTimeoutMs", self.reader_timeout_ms),
            ("maxRows", self.max_rows),
            ("maxBytes", self.max_bytes),
        ] {
            if let Some(n) = v {
                o.insert(k.into(), json!(n));
            }
        }
        if o.is_empty() {
            J::Null
        } else {
            J::Object(o)
        }
    }
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Serve the configured database on stdin and stdout.
    Serve(Config),
    /// Print the usage text.
    Help,
    /// Print the version.
    Version,
}

/// The usage text of the `tiramemsu-mcp` binary.
pub const USAGE: &str = "\
tiramemsu-mcp: a local MCP server (stdio, JSON-RPC 2.0) over one tiramemsu memory file

USAGE:
    tiramemsu-mcp --db <path> [options]

OPTIONS:
    --db <path>               The database file (created if missing). Required.
    --read-only               Refuse assert, confirm, supersede and import_bundle.
    --timeout-ms <n>          Deadline of one tool call (default 30000; 0: none).
    --reader-timeout-ms <n>   Wait for a read connection (default: unbounded).
    --max-rows <n>            Rows one call may decode (default 10000; 0: none).
    --max-bytes <n>           Decoded bytes one call may produce (default 8388608; 0: none).
    --text-index              Build the text index at open, for text_search.
    -h, --help                Print this text.
    -V, --version             Print the version.

REGISTER:
    claude mcp add tiramemsu -- tiramemsu-mcp --db ./memory.db
";

/// Parses the arguments after the program name.
///
/// # Errors
///
/// A message naming the bad argument: an unknown flag, a missing or
/// non-numeric value, a repeated `--db`, or no `--db` at all.
///
/// ```
/// use tiramemsu_mcp::{parse_args, Command};
///
/// let Command::Serve(c) = parse_args(["--db", "m.db", "--read-only", "--max-rows", "0"])? else {
///     unreachable!()
/// };
/// assert!(c.read_only);
/// assert_eq!(c.budget.max_rows, None); // 0 is unbounded
/// assert!(parse_args(["--read-only"]).is_err()); // --db is required
/// # Ok::<(), String>(())
/// ```
pub fn parse_args<I, S>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut db: Option<PathBuf> = None;
    let mut config = Config::new("");
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let a = a.as_ref();
        // `--flag=value` and `--flag value` are the same
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (a, None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => it
                    .next()
                    .map(|v| v.as_ref().to_string())
                    .ok_or_else(|| format!("{name} needs a value")),
            }
        };
        let number = |name: &str, v: String| -> Result<Option<u64>, String> {
            let n: u64 = v
                .parse()
                .map_err(|_| format!("{name} must be a non-negative integer, not {v:?}"))?;
            Ok((n > 0).then_some(n))
        };
        match flag {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--db" => {
                if db.is_some() {
                    return Err("--db is given twice".into());
                }
                let v = value(flag)?;
                if v.is_empty() {
                    return Err("--db needs a path".into());
                }
                db = Some(PathBuf::from(v));
            }
            "--read-only" => config.read_only = true,
            "--text-index" => config.text_index = true,
            "--timeout-ms" => config.budget.timeout_ms = number(flag, value(flag)?)?,
            "--reader-timeout-ms" => config.budget.reader_timeout_ms = number(flag, value(flag)?)?,
            "--max-rows" => config.budget.max_rows = number(flag, value(flag)?)?,
            "--max-bytes" => config.budget.max_bytes = number(flag, value(flag)?)?,
            other => return Err(format!("unknown argument {other:?}")),
        }
        if inline.is_some() && matches!(flag, "--read-only" | "--text-index") {
            return Err(format!("{flag} takes no value"));
        }
    }
    config.db = db.ok_or("--db <path> is required")?;
    Ok(Command::Serve(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    // add-mcp-adapter "Explicit local configuration": flags map to the config
    // @lat: [[tests#MCP Adapter#Command Line Configures The Server]]
    #[test]
    fn flags_map_to_the_config() {
        let Command::Serve(c) = parse_args([
            "--db",
            "/tmp/m.db",
            "--read-only",
            "--timeout-ms=250",
            "--reader-timeout-ms",
            "40",
            "--max-rows",
            "5",
            "--max-bytes",
            "0",
            "--text-index",
        ])
        .unwrap() else {
            panic!("not serve")
        };
        assert_eq!(c.db, PathBuf::from("/tmp/m.db"));
        assert!(c.read_only && c.text_index);
        assert_eq!(
            c.budget,
            Budget {
                timeout_ms: Some(250),
                reader_timeout_ms: Some(40),
                max_rows: Some(5),
                max_bytes: None,
            }
        );
        assert_eq!(
            c.budget.to_json(),
            json!({ "timeoutMs": 250, "readerTimeoutMs": 40, "maxRows": 5 })
        );
        // defaults bound every call
        let Command::Serve(d) = parse_args(["--db", "m.db"]).unwrap() else {
            panic!("not serve")
        };
        assert!(!d.read_only && !d.text_index);
        assert_eq!(d.budget, Budget::default());
        let none = Budget {
            timeout_ms: None,
            reader_timeout_ms: None,
            max_rows: None,
            max_bytes: None,
        };
        assert_eq!(none.to_json(), J::Null);
        assert_eq!(parse_args(["-h"]).unwrap(), Command::Help);
        assert_eq!(parse_args(["--version"]).unwrap(), Command::Version);
        for bad in [
            &["--db"][..],
            &[],
            &["--db", "a", "--db", "b"],
            &["--db", "a", "--max-rows", "-1"],
            &["--db", "a", "--frobnicate"],
            &["--db", "a", "--read-only=yes"],
        ] {
            assert!(parse_args(bad.iter().copied()).is_err(), "{bad:?}");
        }
    }
}
