//! `tiramemsu-mcp`: a local MCP server over stdio. See the crate documentation.

use std::process::ExitCode;

use tiramemsu_mcp::{parse_args, Command, Server, USAGE};

fn main() -> ExitCode {
    let config = match parse_args(std::env::args().skip(1)) {
        Ok(Command::Serve(c)) => c,
        Ok(Command::Help) => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("tiramemsu-mcp {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            // stdout carries the protocol only; diagnostics go to stderr
            eprintln!("tiramemsu-mcp: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut server = match Server::open(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("tiramemsu-mcp: cannot open the database: {e}");
            return ExitCode::FAILURE;
        }
    };
    match server.serve(std::io::stdin().lock(), std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tiramemsu-mcp: {e}");
            ExitCode::FAILURE
        }
    }
}
