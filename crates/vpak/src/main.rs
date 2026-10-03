//! `vpak`: intent-based software packaging.
//!
//! Exit codes: 0 ok, 1 error, 2 usage, 3 a human must answer questions.

mod cli;
mod commands;
mod out;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    let code = match commands::dispatch(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("vpak: {e:#}");
            1
        }
    };
    std::process::exit(code);
}
