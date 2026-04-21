pub mod app;
pub mod archive;
pub mod cli;
pub mod config;
pub mod error;
pub mod llm;
pub mod output;
pub mod runner;
pub mod scoring;
pub mod util;

pub use cli::Cli;

pub fn run(cli: Cli) -> anyhow::Result<()> {
    app::run(cli)
}
