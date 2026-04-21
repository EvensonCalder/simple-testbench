use std::path::Path;

use anyhow::Context;

use crate::{
    cli::{Cli, Command, PackageArgs, TestArgs},
    error::StbError,
};

pub fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Command::Test(args) => run_test(args),
        Command::Mkt(args) => run_package("test", args),
        Command::Mks(args) => run_package("scoring", args),
    }
}

fn run_test(args: TestArgs) -> anyhow::Result<()> {
    ensure_path_exists(&args.input)?;

    if let Some(path) = &args.test_archive {
        ensure_path_exists(path)?;
    }

    if let Some(path) = &args.score_archive {
        ensure_path_exists(path)?;
    }

    if args.dry_run {
        print_test_dry_run(&args);
        return Ok(());
    }

    Err(StbError::NotImplemented("benchmark execution").into())
}

fn run_package(kind: &'static str, args: PackageArgs) -> anyhow::Result<()> {
    ensure_path_exists(&args.input)?;

    println!(
        "Packaging {kind} inputs from {} into {} is not implemented yet.",
        display_path(&args.input),
        display_path(&args.output),
    );

    Ok(())
}

fn print_test_dry_run(args: &TestArgs) {
    println!("STB dry run");
    println!("input: {}", display_path(&args.input));
    println!(
        "test archive: {}",
        args.test_archive
            .as_deref()
            .map(display_path)
            .unwrap_or("<loose files>")
    );
    println!(
        "score archive: {}",
        args.score_archive
            .as_deref()
            .map(display_path)
            .unwrap_or("<loose files>")
    );
    println!("retry: {}", args.retry);
    println!(
        "provider filter: {}",
        args.provider.as_deref().unwrap_or("<all providers>")
    );
    println!(
        "model filter: {}",
        args.model.as_deref().unwrap_or("<all models>")
    );
    println!(
        "repeat override: {}",
        args.repeat
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<use tests.json>".to_string())
    );
    println!(
        "global concurrency override: {}",
        args.concurrency
            .map(|value| value.to_string())
            .unwrap_or_else(|| "<use provider limits>".to_string())
    );
    println!(
        "output dir: {}",
        args.output_dir
            .as_deref()
            .map(display_path)
            .unwrap_or("<auto timestamped>")
    );
    println!("write results.json: {}", args.json);
    println!("verbose: {}", args.verbose);
    println!("fresh: {}", args.fresh);
    println!("post-process disabled: {}", args.disable_post_process);
    println!(
        "display format: {}",
        args.format
            .map(display_format_name)
            .unwrap_or_else(|| "table".to_string())
    );
    println!("planner status: pending implementation");
}

fn display_format_name(format: crate::cli::DisplayFormat) -> String {
    match format {
        crate::cli::DisplayFormat::Table => "table".to_string(),
        crate::cli::DisplayFormat::Json => "json".to_string(),
    }
}

fn ensure_path_exists(path: &Path) -> anyhow::Result<()> {
    path.try_exists()
        .with_context(|| format!("failed to access {}", display_path(path)))?
        .then_some(())
        .ok_or_else(|| StbError::MissingPath(path.to_path_buf()).into())
}

fn display_path(path: &Path) -> &str {
    path.to_str().unwrap_or("<non-utf8 path>")
}
