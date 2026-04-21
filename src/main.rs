use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    let cli = stb::Cli::parse();

    match stb::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
