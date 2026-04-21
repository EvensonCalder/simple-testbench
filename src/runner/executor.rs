use std::collections::{HashMap, HashSet};

use anyhow::anyhow;

use crate::{
    cli::TestArgs,
    config::{ApiStyle, LoadedConfig, SystemPrompt, resolve_selected_models},
    error::StbError,
    llm::openai_chat::{RetryPolicy, execute_openai_chat_completion},
    output::{self, ExecutionRecord, RecordStatus, RunOutput},
};

#[derive(Debug, Clone)]
pub struct ExecutionSummary {
    pub output_dir: std::path::PathBuf,
    pub completed_requests: usize,
    pub failed_requests: usize,
    pub skipped_requests: usize,
    pub resumed_requests: usize,
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
    let unsupported = selected
        .iter()
        .filter(|resolved| resolved.model.api_style != ApiStyle::OpenaiChatCompletions)
        .map(|resolved| {
            format!(
                "{}/{} [{}]",
                resolved.provider.provider_id,
                resolved.model.model_id,
                resolved.model.api_style.as_str()
            )
        })
        .collect::<Vec<_>>();

    if !unsupported.is_empty() {
        return Err(StbError::NotImplemented(
            "only openai_chat_completions execution is implemented so far",
        )
        .into());
    }

    let output_dir = output::prepare_output_dir(args.output_dir.as_deref(), args.fresh)?;
    let output_path = output::output_json_path(&output_dir);
    let system_prompt_index = system_prompt_index(loaded);
    let retry_policy = RetryPolicy::from_retry_count(args.retry);
    let mut run_output = if args.fresh || !output_path.exists() {
        RunOutput::new()
    } else {
        output::load_run_output(&output_path)?
    };
    let mut existing_records = run_output
        .records
        .iter()
        .map(ExecutionRecord::key)
        .collect::<HashSet<_>>();
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

                if existing_records.contains(&record_key) {
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
                        error: Some(reason.clone()),
                    };
                    existing_records.insert(record.key());
                    run_output.records.push(record);
                    output::write_run_output(&output_path, &run_output)?;
                    skipped_requests += 1;
                    continue;
                }

                match execute_openai_chat_completion(
                    resolved.provider,
                    resolved.model,
                    system_prompt,
                    test_case,
                    &retry_policy,
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
                            error: None,
                        };
                        existing_records.insert(record.key());
                        run_output.records.push(record);
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
                            attempts: args.retry + 1,
                            output_text: None,
                            error: Some(rendered.clone()),
                        };
                        existing_records.insert(record.key());
                        run_output.records.push(record);
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

    Ok(ExecutionSummary {
        output_dir,
        completed_requests,
        failed_requests,
        skipped_requests,
        resumed_requests,
    })
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
