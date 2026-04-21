use std::path::Path;

use anyhow::Context;

use crate::{
    cli::{Cli, Command, PackageArgs, TestArgs},
    config,
    error::StbError,
    runner::planner,
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

    let loaded = config::load_config(&args.input, args.test_archive.as_deref())?;

    if args.dry_run {
        let plan = planner::build_dry_run_plan(&loaded, &args)?;
        print_test_dry_run(&args, &loaded, &plan);
        return Ok(());
    }

    Err(StbError::NotImplemented("benchmark execution").into())
}

fn run_package(kind: &'static str, args: PackageArgs) -> anyhow::Result<()> {
    ensure_path_exists(&args.input)?;

    let bundle = match kind {
        "test" => crate::archive::package_test_bundle(&args.input, &args.output)?,
        "scoring" => crate::archive::package_scoring_bundle(&args.input, &args.output)?,
        _ => return Err(StbError::NotImplemented("unknown package kind").into()),
    };

    println!(
        "Packaged {} bundle to {}",
        bundle.kind,
        display_path(&args.output)
    );
    println!("included files: {}", bundle.files.join(", "));

    Ok(())
}

fn print_test_dry_run(args: &TestArgs, loaded: &config::LoadedConfig, plan: &planner::DryRunPlan) {
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
    println!("loaded providers: {}", loaded.providers.len());
    println!("loaded models: {}", loaded.models.len());
    println!("loaded system prompts: {}", loaded.system_prompts.len());
    println!("loaded tests: {}", loaded.tests.len());
    println!("selected providers: {}", plan.provider_count);
    println!("selected model instances: {}", plan.selected_model_count);
    println!("total tests: {}", plan.test_count);
    println!("total repeat units: {}", plan.total_repeats);
    println!("planned requests: {}", plan.planned_requests);

    for model in &plan.selected_models {
        println!(
            "- {}/{} [{}] endpoint={} provider_concurrency={} effective_concurrency={} rpm={} planned_requests={}",
            model.provider_id,
            model.model_id,
            model.api_style,
            model.endpoint,
            model.configured_concurrency,
            model.effective_concurrency,
            model.rpm,
            model.planned_requests,
        );
    }
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
