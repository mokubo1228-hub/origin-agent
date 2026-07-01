mod args;
mod chat_workflow;
mod commands;
mod edit_workflow;
mod runtime;
mod shell;
mod status_snapshot;
mod tui;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args = match args::Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    match commands::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
