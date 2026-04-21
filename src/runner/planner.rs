use crate::{
    cli::TestArgs,
    config::{LoadedConfig, ResolvedModel, resolve_selected_models},
};

#[derive(Debug, Clone)]
pub struct DryRunPlan {
    pub selected_model_count: usize,
    pub provider_count: usize,
    pub test_count: usize,
    pub total_repeats: u64,
    pub planned_requests: u64,
    pub selected_models: Vec<PlannedModel>,
}

#[derive(Debug, Clone)]
pub struct PlannedModel {
    pub provider_id: String,
    pub model_id: String,
    pub api_style: String,
    pub endpoint: String,
    pub configured_concurrency: u32,
    pub effective_concurrency: usize,
    pub rpm: u32,
    pub planned_requests: u64,
}

pub fn build_dry_run_plan(loaded: &LoadedConfig, args: &TestArgs) -> anyhow::Result<DryRunPlan> {
    let selected_models =
        resolve_selected_models(loaded, args.provider.as_deref(), args.model.as_deref())?;

    let total_repeats = total_repeats(loaded, args.repeat);
    let provider_count = selected_provider_count(&selected_models);
    let planned_models = selected_models
        .iter()
        .map(|resolved| build_planned_model(resolved, args.concurrency, total_repeats))
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(DryRunPlan {
        selected_model_count: planned_models.len(),
        provider_count,
        test_count: loaded.tests.len(),
        total_repeats,
        planned_requests: planned_models
            .iter()
            .map(|model| model.planned_requests)
            .sum(),
        selected_models: planned_models,
    })
}

fn build_planned_model(
    resolved: &ResolvedModel<'_>,
    global_concurrency_override: Option<usize>,
    total_repeats: u64,
) -> anyhow::Result<PlannedModel> {
    let endpoint = resolved
        .provider
        .endpoints
        .endpoint_for(&resolved.model.api_style)
        .ok_or_else(|| {
            crate::error::StbError::InvalidConfig(format!(
                "provider {} does not define endpoint {} required by model {}",
                resolved.provider.provider_id,
                resolved.model.api_style.as_str(),
                resolved.model.model_id
            ))
        })?;

    let provider_concurrency = resolved.provider.concurrency as usize;
    let effective_concurrency = global_concurrency_override
        .map(|value| value.min(provider_concurrency))
        .unwrap_or(provider_concurrency);

    Ok(PlannedModel {
        provider_id: resolved.provider.provider_id.clone(),
        model_id: resolved.model.model_id.clone(),
        api_style: resolved.model.api_style.as_str().to_string(),
        endpoint: endpoint.to_string(),
        configured_concurrency: resolved.provider.concurrency,
        effective_concurrency,
        rpm: resolved.provider.rpm,
        planned_requests: total_repeats,
    })
}

fn total_repeats(loaded: &LoadedConfig, repeat_override: Option<u32>) -> u64 {
    loaded
        .tests
        .iter()
        .map(|test| u64::from(repeat_override.unwrap_or(test.repeat)))
        .sum()
}

fn selected_provider_count(selected_models: &[ResolvedModel<'_>]) -> usize {
    let mut providers = std::collections::BTreeSet::new();
    for resolved in selected_models {
        providers.insert(resolved.provider.provider_id.as_str());
    }
    providers.len()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::{
        cli::TestArgs,
        config::{
            ApiStyle, LoadedConfig, ModelConfig, ProviderConfig, ProviderEndpoints, SystemPrompt,
            TestCase, TestInput,
        },
    };

    use super::build_dry_run_plan;

    fn args() -> TestArgs {
        TestArgs {
            test_archive: None,
            score_archive: None,
            input: Path::new(".").to_path_buf(),
            retry: 3,
            provider: None,
            model: None,
            json: false,
            verbose: false,
            repeat: None,
            concurrency: None,
            dry_run: true,
            fresh: false,
            output_dir: None,
            disable_post_process: false,
            format: None,
        }
    }

    fn loaded_config() -> LoadedConfig {
        LoadedConfig {
            input_dir: Path::new(".").to_path_buf(),
            providers: vec![ProviderConfig {
                provider_id: "openrouter".to_string(),
                key: None,
                env_key: Some("OPENROUTER_API_KEY".to_string()),
                concurrency: 4,
                rpm: 60,
                endpoints: ProviderEndpoints {
                    openai_chat_completions: Some("https://openrouter.ai/api/v1".to_string()),
                    openai_responses: Some("https://openrouter.ai/api/v1/responses".to_string()),
                    anthropic_messages: Some("https://openrouter.ai/api/v1/messages".to_string()),
                },
            }],
            models: vec![
                ModelConfig {
                    provider_id: "openrouter".to_string(),
                    model_id: "model-a".to_string(),
                    api_style: ApiStyle::OpenaiChatCompletions,
                    temperature: Some(0.0),
                    max_output_tokens: Some(512),
                    extra: Default::default(),
                },
                ModelConfig {
                    provider_id: "openrouter".to_string(),
                    model_id: "model-b".to_string(),
                    api_style: ApiStyle::OpenaiResponses,
                    temperature: Some(0.0),
                    max_output_tokens: Some(512),
                    extra: Default::default(),
                },
            ],
            system_prompts: vec![SystemPrompt {
                id: "prompt-a".to_string(),
                text: "Return JSON".to_string(),
            }],
            tests: vec![
                TestCase {
                    id: "test-1".to_string(),
                    system_prompt: "prompt-a".to_string(),
                    input: vec![TestInput::Text {
                        text: "buy milk".to_string(),
                    }],
                    repeat: 1,
                },
                TestCase {
                    id: "test-2".to_string(),
                    system_prompt: "prompt-a".to_string(),
                    input: vec![TestInput::Text {
                        text: "call Alice".to_string(),
                    }],
                    repeat: 2,
                },
            ],
        }
    }

    #[test]
    fn computes_requests_from_tests_and_models() {
        let plan = build_dry_run_plan(&loaded_config(), &args()).expect("plan should build");

        assert_eq!(plan.selected_model_count, 2);
        assert_eq!(plan.test_count, 2);
        assert_eq!(plan.total_repeats, 3);
        assert_eq!(plan.planned_requests, 6);
    }

    #[test]
    fn applies_repeat_and_concurrency_overrides() {
        let mut args = args();
        args.repeat = Some(5);
        args.concurrency = Some(2);

        let plan = build_dry_run_plan(&loaded_config(), &args).expect("plan should build");

        assert_eq!(plan.total_repeats, 10);
        assert_eq!(plan.planned_requests, 20);
        assert!(
            plan.selected_models
                .iter()
                .all(|model| model.effective_concurrency == 2)
        );
    }
}
