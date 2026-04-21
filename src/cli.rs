use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "stb",
    version,
    about = "Structured task bench for LLM evaluation",
    long_about = "STB benchmarks model outputs against test suites, post-processors, and scoring pipelines."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    /// Run a benchmark session.
    Test(TestArgs),
    /// Package test input files into a .stbt archive.
    Mkt(PackageArgs),
    /// Package scoring files into a .stbs archive.
    Mks(PackageArgs),
}

#[derive(Debug, Clone, Args)]
pub struct PackageArgs {
    /// Input directory containing loose files.
    #[arg(short = 'i', long = "input", default_value = ".")]
    pub input: PathBuf,

    /// Output archive path.
    #[arg(short = 'o', long = "output")]
    pub output: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct TestArgs {
    /// Test archive path.
    #[arg(short = 't', long = "test-archive")]
    pub test_archive: Option<PathBuf>,

    /// Scoring archive path.
    #[arg(short = 's', long = "score-archive")]
    pub score_archive: Option<PathBuf>,

    /// Working directory containing loose files.
    #[arg(short = 'i', long = "input", default_value = ".")]
    pub input: PathBuf,

    /// Request retry count.
    #[arg(long = "retry", default_value_t = 3, value_parser = clap::value_parser!(u8).range(0..=3))]
    pub retry: u8,

    /// Restrict execution to one provider.
    #[arg(long = "provider")]
    pub provider: Option<String>,

    /// Restrict execution to one model. Requires --provider.
    #[arg(long = "model", requires = "provider")]
    pub model: Option<String>,

    /// Write results.json in addition to CSV outputs.
    #[arg(long = "json")]
    pub json: bool,

    /// Print detailed request and response information.
    #[arg(long = "verbose")]
    pub verbose: bool,

    /// Override repeat count for all tests.
    #[arg(long = "repeat", value_parser = clap::value_parser!(u32).range(1..))]
    pub repeat: Option<u32>,

    /// Override global concurrency.
    #[arg(long = "concurrency")]
    pub concurrency: Option<usize>,

    /// Print the resolved execution plan without running requests.
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Ignore any existing output.json state and start over.
    #[arg(long = "fresh")]
    pub fresh: bool,

    /// Directory for generated outputs.
    #[arg(long = "output-dir")]
    pub output_dir: Option<PathBuf>,

    /// Disable post-processing.
    #[arg(long = "npp")]
    pub disable_post_process: bool,

    /// Optional future output format selector for CLI display.
    #[arg(long = "format")]
    pub format: Option<DisplayFormat>,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum DisplayFormat {
    Table,
    Json,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Cli, Command, DisplayFormat};

    #[test]
    fn parses_test_command_with_filters() {
        let cli = Cli::try_parse_from([
            "stb",
            "test",
            "--provider",
            "openrouter",
            "--model",
            "z-ai/glm-5.1",
            "--retry",
            "2",
            "--repeat",
            "5",
            "--dry-run",
            "--format",
            "json",
        ])
        .expect("test args should parse");

        match cli.command {
            Command::Test(args) => {
                assert_eq!(args.provider.as_deref(), Some("openrouter"));
                assert_eq!(args.model.as_deref(), Some("z-ai/glm-5.1"));
                assert_eq!(args.retry, 2);
                assert_eq!(args.repeat, Some(5));
                assert!(args.dry_run);
                assert_eq!(args.format, Some(DisplayFormat::Json));
            }
            _ => panic!("expected test command"),
        }
    }

    #[test]
    fn rejects_model_without_provider() {
        let error = Cli::try_parse_from(["stb", "test", "--model", "z-ai/glm-5.1"])
            .expect_err("model without provider should fail");

        let rendered = error.to_string();
        assert!(rendered.contains("--provider"));
    }

    #[test]
    fn rejects_retry_out_of_range() {
        let error = Cli::try_parse_from(["stb", "test", "--retry", "4"])
            .expect_err("retry > 3 should fail");

        assert!(error.to_string().contains("0..=3"));
    }
}
