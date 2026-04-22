use std::collections::{HashMap, HashSet};

use anyhow::anyhow;

use crate::{
    cli::TestArgs,
    config::{LoadedConfig, SystemPrompt, resolve_selected_models},
    error::StbError,
    llm::{self, RetryPolicy},
    output::{self, ExecutionRecord, RecordStatus, ReportArtifacts, RunOutput},
    scoring,
};

#[derive(Debug, Clone)]
pub struct ExecutionSummary {
    pub output_dir: std::path::PathBuf,
    pub completed_requests: usize,
    pub failed_requests: usize,
    pub skipped_requests: usize,
    pub resumed_requests: usize,
    pub reports: ReportArtifacts,
}

pub fn run_test_session(
    loaded: &LoadedConfig,
    args: &TestArgs,
) -> anyhow::Result<ExecutionSummary> {
    if loaded.tests.is_empty() {
        return Err(
            StbError::InvalidConfig("no tests are configured for execution".to_string()).into(),
        );
    }

    let selected =
        resolve_selected_models(loaded, args.provider.as_deref(), args.model.as_deref())?;
    let output_dir = output::prepare_output_dir(args.output_dir.as_deref(), args.fresh)?;
    let output_path = output::output_json_path(&output_dir);
    let system_prompt_index = system_prompt_index(loaded);
    let scoring_config = scoring::load_scoring_config(&args.input, args.score_archive.as_deref())?;
    let retry_policy = RetryPolicy::from_retry_count(args.retry);
    let mut run_output = if args.fresh || !output_path.exists() {
        RunOutput::new()
    } else {
        output::load_run_output(&output_path)?
    };
    let mut existing_records = run_output
        .records
        .iter()
        .enumerate()
        .map(|(index, record)| (record.key(), index))
        .collect::<HashMap<_, _>>();
    let mut disabled_models = disabled_models(&run_output);
    let mut completed_requests = 0usize;
    let mut failed_requests = 0usize;
    let mut skipped_requests = 0usize;
    let mut resumed_requests = 0usize;

    for resolved in selected {
        let model_key = (
            resolved.provider.provider_id.clone(),
            resolved.model.model_id.clone(),
        );
        let mut disabled_reason = disabled_models.get(&model_key).cloned();

        for test_case in &loaded.tests {
            let system_prompt = system_prompt_index
                .get(test_case.system_prompt.as_str())
                .copied()
                .ok_or_else(|| {
                    anyhow!(
                        "missing system prompt {} for test {}",
                        test_case.system_prompt,
                        test_case.id
                    )
                })?;

            let repeat_count = args.repeat.unwrap_or(test_case.repeat);
            for repeat_index in 1..=repeat_count {
                let record_key = output::RecordKey {
                    provider_id: resolved.provider.provider_id.clone(),
                    model_id: resolved.model.model_id.clone(),
                    test_id: test_case.id.clone(),
                    repeat_index,
                };

                if let Some(index) = existing_records.get(&record_key).copied() {
                    if maybe_complete_existing_record(
                        &mut run_output.records[index],
                        loaded,
                        test_case,
                        &scoring_config,
                        &retry_policy,
                        args.disable_post_process,
                        args.verbose,
                    )? {
                        output::write_run_output(&output_path, &run_output)?;
                    }
                    resumed_requests += 1;
                    continue;
                }

                if let Some(reason) = &disabled_reason {
                    let record = ExecutionRecord {
                        id: output::next_record_id(),
                        provider_id: resolved.provider.provider_id.clone(),
                        model_id: resolved.model.model_id.clone(),
                        test_id: test_case.id.clone(),
                        repeat_index,
                        api_style: resolved.model.api_style.as_str().to_string(),
                        status: RecordStatus::SkippedModelDisabled,
                        attempts: 0,
                        output_text: None,
                        processed_output: None,
                        post_process_applied: false,
                        post_process_retries: 0,
                        scores: Vec::new(),
                        error: Some(reason.clone()),
                    };
                    let record_index = run_output.records.len();
                    run_output.records.push(record);
                    existing_records.insert(record_key, record_index);
                    output::write_run_output(&output_path, &run_output)?;
                    skipped_requests += 1;
                    continue;
                }

                match execute_request_with_scoring(
                    loaded,
                    resolved.provider,
                    resolved.model,
                    system_prompt,
                    test_case,
                    &retry_policy,
                    &scoring_config,
                    args.disable_post_process,
                    args.verbose,
                ) {
                    Ok(execution) => {
                        let record = ExecutionRecord {
                            id: output::next_record_id(),
                            provider_id: resolved.provider.provider_id.clone(),
                            model_id: resolved.model.model_id.clone(),
                            test_id: test_case.id.clone(),
                            repeat_index,
                            api_style: resolved.model.api_style.as_str().to_string(),
                            status: RecordStatus::Success,
                            attempts: execution.attempts,
                            output_text: Some(execution.output_text),
                            processed_output: Some(execution.processed_output),
                            post_process_applied: execution.post_process_applied,
                            post_process_retries: execution.post_process_retries,
                            scores: execution.scores,
                            error: None,
                        };
                        let record_index = run_output.records.len();
                        run_output.records.push(record);
                        existing_records.insert(record_key, record_index);
                        output::write_run_output(&output_path, &run_output)?;
                        completed_requests += 1;
                    }
                    Err(error) => {
                        let rendered = error.to_string();
                        let record = ExecutionRecord {
                            id: output::next_record_id(),
                            provider_id: resolved.provider.provider_id.clone(),
                            model_id: resolved.model.model_id.clone(),
                            test_id: test_case.id.clone(),
                            repeat_index,
                            api_style: resolved.model.api_style.as_str().to_string(),
                            status: RecordStatus::Failed,
                            attempts: u32::from(args.retry) + 1,
                            output_text: None,
                            processed_output: None,
                            post_process_applied: false,
                            post_process_retries: 0,
                            scores: Vec::new(),
                            error: Some(rendered.clone()),
                        };
                        let record_index = run_output.records.len();
                        run_output.records.push(record);
                        existing_records.insert(record_key, record_index);
                        output::write_run_output(&output_path, &run_output)?;
                        failed_requests += 1;
                        disabled_reason = Some(rendered);
                        if let Some(reason) = &disabled_reason {
                            disabled_models.insert(model_key.clone(), reason.clone());
                        }
                    }
                }
            }
        }
    }

    let reports = output::write_reports(&output_dir, &run_output, args.json)?;

    Ok(ExecutionSummary {
        output_dir,
        completed_requests,
        failed_requests,
        skipped_requests,
        resumed_requests,
        reports,
    })
}

#[derive(Debug)]
struct CompletedExecution {
    output_text: String,
    processed_output: String,
    attempts: u32,
    post_process_applied: bool,
    post_process_retries: u32,
    scores: Vec<crate::output::ScoreResult>,
}

fn execute_request_with_scoring(
    loaded: &LoadedConfig,
    provider: &crate::config::ProviderConfig,
    model: &crate::config::ModelConfig,
    system_prompt: &SystemPrompt,
    test_case: &crate::config::TestCase,
    retry_policy: &RetryPolicy,
    scoring_config: &scoring::LoadedScoringConfig,
    disable_post_process: bool,
    verbose: bool,
) -> anyhow::Result<CompletedExecution> {
    let mut total_attempts = 0u32;
    let mut post_process_retries = 0u32;
    let request_label = format!("test={}", test_case.id);
    let user_prompt = llm::build_test_user_prompt(test_case)?;

    loop {
        let execution = llm::execute_model_request(
            provider,
            model,
            &system_prompt.text,
            &user_prompt,
            retry_policy,
            verbose,
            &request_label,
        )?;
        total_attempts += u32::from(execution.attempts);

        let (processed_output, post_process_applied, should_retry, max_retry) =
            post_process_output(&execution.output_text, scoring_config, disable_post_process)?;

        if should_retry && post_process_retries < max_retry {
            post_process_retries += 1;

            if verbose {
                println!(
                    "post-process requested retry for {}/{} test={} retry={}/{}",
                    provider.provider_id,
                    model.model_id,
                    test_case.id,
                    post_process_retries,
                    max_retry,
                );
            }

            continue;
        }

        let scores = scoring::score_processed_output(
            scoring_config,
            loaded,
            test_case,
            &processed_output,
            retry_policy,
            verbose,
        );

        return Ok(CompletedExecution {
            output_text: execution.output_text,
            processed_output,
            attempts: total_attempts,
            post_process_applied,
            post_process_retries,
            scores,
        });
    }
}

fn maybe_complete_existing_record(
    record: &mut ExecutionRecord,
    loaded: &LoadedConfig,
    test_case: &crate::config::TestCase,
    scoring_config: &scoring::LoadedScoringConfig,
    retry_policy: &RetryPolicy,
    disable_post_process: bool,
    verbose: bool,
) -> anyhow::Result<bool> {
    if record.status != RecordStatus::Success {
        return Ok(false);
    }

    let Some(raw_output) = record.output_text.as_deref() else {
        return Ok(false);
    };

    let mut changed = false;

    if record.processed_output.is_none() {
        let (processed_output, post_process_applied, _, _) =
            post_process_output(raw_output, scoring_config, disable_post_process)?;
        record.processed_output = Some(processed_output);
        record.post_process_applied = post_process_applied;
        record.post_process_retries = 0;
        changed = true;
    }

    if !has_all_scores(record, scoring_config) {
        let processed_output = record.processed_output.as_deref().unwrap_or(raw_output);

        for score in scoring::score_processed_output(
            scoring_config,
            loaded,
            test_case,
            processed_output,
            retry_policy,
            verbose,
        ) {
            if record
                .scores
                .iter()
                .any(|existing| existing.name == score.name)
            {
                continue;
            }

            record.scores.push(score);
            changed = true;
        }
    }

    Ok(changed)
}

fn post_process_output(
    raw_output: &str,
    scoring_config: &scoring::LoadedScoringConfig,
    disable_post_process: bool,
) -> anyhow::Result<(String, bool, bool, u32)> {
    if disable_post_process {
        return Ok((raw_output.to_string(), false, false, 0));
    }

    let Some(script) = scoring_config.post_process.as_deref() else {
        return Ok((raw_output.to_string(), false, false, 0));
    };

    let outcome = scoring::apply_post_process(script, raw_output)?;
    Ok((outcome.output, true, outcome.retry, outcome.max_retry))
}

fn has_all_scores(record: &ExecutionRecord, scoring_config: &scoring::LoadedScoringConfig) -> bool {
    if scoring_config.scorers.is_empty() {
        return true;
    }

    let existing = record
        .scores
        .iter()
        .map(|score| score.name.as_str())
        .collect::<HashSet<_>>();

    scoring_config
        .scorer_names()
        .into_iter()
        .all(|name| existing.contains(name))
}

fn system_prompt_index(loaded: &LoadedConfig) -> HashMap<&str, &SystemPrompt> {
    loaded
        .system_prompts
        .iter()
        .map(|prompt| (prompt.id.as_str(), prompt))
        .collect()
}

fn disabled_models(run_output: &RunOutput) -> HashMap<(String, String), String> {
    run_output
        .records
        .iter()
        .filter(|record| record.status == RecordStatus::Failed)
        .filter_map(|record| {
            record.error.as_ref().map(|error| {
                (
                    (record.provider_id.clone(), record.model_id.clone()),
                    error.clone(),
                )
            })
        })
        .collect()
}
