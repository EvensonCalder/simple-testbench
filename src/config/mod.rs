//! Configuration loading and validation.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::archive;
use crate::error::StbError;

const PROVIDERS_FILE: &str = "providers.json";
const MODELS_FILE: &str = "models.json";
const SYSTEM_PROMPTS_FILE: &str = "system_prompts.json";
const TESTS_FILE: &str = "tests.json";
pub const DEFAULT_MODEL_TIMEOUT_SECONDS: u64 = 300;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ApiStyle {
    OpenaiChatCompletions,
    OpenaiResponses,
    AnthropicMessages,
}

impl ApiStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenaiChatCompletions => "openai_chat_completions",
            Self::OpenaiResponses => "openai_responses",
            Self::AnthropicMessages => "anthropic_messages",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProvidersFile {
    pub providers: Vec<ProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderConfig {
    pub provider_id: String,
    pub key: Option<String>,
    pub env_key: Option<String>,
    pub concurrency: u32,
    pub rpm: u32,
    pub endpoints: ProviderEndpoints,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProviderEndpoints {
    pub openai_chat_completions: Option<String>,
    pub openai_responses: Option<String>,
    pub anthropic_messages: Option<String>,
}

impl ProviderEndpoints {
    pub fn endpoint_for(&self, api_style: &ApiStyle) -> Option<&str> {
        match api_style {
            ApiStyle::OpenaiChatCompletions => self.openai_chat_completions.as_deref(),
            ApiStyle::OpenaiResponses => self.openai_responses.as_deref(),
            ApiStyle::AnthropicMessages => self.anthropic_messages.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelsFile {
    pub models: Vec<ModelConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub provider_id: String,
    pub model_id: String,
    pub api_style: ApiStyle,
    pub temperature: Option<f64>,
    pub max_output_tokens: Option<u32>,
    #[serde(default = "default_streaming")]
    pub streaming: bool,
    #[serde(default = "default_model_timeout_seconds")]
    pub timeout: u64,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl ModelConfig {
    pub fn config_key(&self) -> String {
        #[derive(Serialize)]
        struct ConfigKey<'a> {
            api_style: &'a ApiStyle,
            temperature: Option<f64>,
            max_output_tokens: Option<u32>,
            streaming: bool,
            timeout: u64,
            extra: &'a BTreeMap<String, Value>,
        }

        serde_json::to_string(&ConfigKey {
            api_style: &self.api_style,
            temperature: self.temperature,
            max_output_tokens: self.max_output_tokens,
            streaming: self.streaming,
            timeout: self.timeout,
            extra: &self.extra,
        })
        .expect("model config key serialization should succeed")
    }

    pub fn instance_id(&self) -> String {
        let identity = format!(
            "{}:{}:{}",
            self.provider_id,
            self.model_id,
            self.config_key()
        );

        Uuid::new_v5(&Uuid::NAMESPACE_OID, identity.as_bytes()).to_string()
    }

    pub fn short_instance_id(&self) -> String {
        self.instance_id().chars().take(8).collect()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemPromptsFile {
    pub system_prompts: Vec<SystemPrompt>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SystemPrompt {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestsFile {
    pub tests: Vec<TestCase>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TestCase {
    pub id: String,
    pub system_prompt: String,
    pub input: Vec<TestInput>,
    #[serde(default = "default_repeat")]
    pub repeat: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestInput {
    Text { text: String },
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub input_dir: PathBuf,
    pub providers: Vec<ProviderConfig>,
    pub models: Vec<ModelConfig>,
    pub system_prompts: Vec<SystemPrompt>,
    pub tests: Vec<TestCase>,
}

pub fn load_config(input_dir: &Path, test_archive: Option<&Path>) -> anyhow::Result<LoadedConfig> {
    let providers = read_json_file::<ProvidersFile>(&input_dir.join(PROVIDERS_FILE))?.providers;
    let models = read_json_file::<ModelsFile>(&input_dir.join(MODELS_FILE))?.models;

    let (system_prompts, tests) = if let Some(path) = test_archive {
        let bundle = archive::load_test_bundle(path)?;
        (
            parse_json_str::<SystemPromptsFile>(&bundle.system_prompts_json, SYSTEM_PROMPTS_FILE)?
                .system_prompts,
            parse_json_str::<TestsFile>(&bundle.tests_json, TESTS_FILE)?.tests,
        )
    } else {
        (
            read_optional_json_file::<SystemPromptsFile>(&input_dir.join(SYSTEM_PROMPTS_FILE))?
                .map(|file| file.system_prompts)
                .unwrap_or_default(),
            read_optional_json_file::<TestsFile>(&input_dir.join(TESTS_FILE))?
                .map(|file| file.tests)
                .unwrap_or_default(),
        )
    };

    let loaded = LoadedConfig {
        input_dir: input_dir.to_path_buf(),
        providers,
        models,
        system_prompts,
        tests,
    };

    validate_loaded_config(&loaded)?;

    Ok(loaded)
}

pub fn load_loose_config(input_dir: &Path) -> anyhow::Result<LoadedConfig> {
    load_config(input_dir, None)
}

pub fn resolve_selected_models<'a>(
    loaded: &'a LoadedConfig,
    provider_filter: Option<&str>,
    model_filter: Option<&str>,
) -> anyhow::Result<Vec<ResolvedModel<'a>>> {
    let provider_index = provider_index(loaded);

    let selected = loaded
        .models
        .iter()
        .filter(|model| provider_filter.is_none_or(|provider_id| model.provider_id == provider_id))
        .filter(|model| model_filter.is_none_or(|model_id| model.model_id == model_id))
        .map(|model| {
            let provider = provider_index
                .get(model.provider_id.as_str())
                .copied()
                .ok_or_else(|| {
                    StbError::InvalidConfig(format!(
                        "model {} references missing provider {}",
                        model.model_id, model.provider_id
                    ))
                })?;

            Ok(ResolvedModel { provider, model })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    if selected.is_empty() {
        return Err(StbError::NoModelsSelected.into());
    }

    Ok(selected)
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedModel<'a> {
    pub provider: &'a ProviderConfig,
    pub model: &'a ModelConfig,
}

fn validate_loaded_config(loaded: &LoadedConfig) -> anyhow::Result<()> {
    let mut provider_ids = BTreeSet::new();
    for provider in &loaded.providers {
        if provider.provider_id.trim().is_empty() {
            return Err(
                StbError::InvalidConfig("provider_id must not be empty".to_string()).into(),
            );
        }

        if !provider_ids.insert(provider.provider_id.clone()) {
            return Err(StbError::InvalidConfig(format!(
                "duplicate provider_id {}",
                provider.provider_id
            ))
            .into());
        }

        if provider.key.is_none() && provider.env_key.is_none() {
            return Err(StbError::InvalidConfig(format!(
                "provider {} must define key or env_key",
                provider.provider_id
            ))
            .into());
        }

        if provider.concurrency == 0 {
            return Err(StbError::InvalidConfig(format!(
                "provider {} concurrency must be greater than zero",
                provider.provider_id
            ))
            .into());
        }

        if provider.rpm == 0 {
            return Err(StbError::InvalidConfig(format!(
                "provider {} rpm must be greater than zero",
                provider.provider_id
            ))
            .into());
        }
    }

    let provider_index = provider_index(loaded);
    let mut model_keys = BTreeSet::new();

    for model in &loaded.models {
        if model.provider_id.trim().is_empty() || model.model_id.trim().is_empty() {
            return Err(StbError::InvalidConfig(
                "model provider_id and model_id must not be empty".to_string(),
            )
            .into());
        }

        if !model_keys.insert((
            model.provider_id.clone(),
            model.model_id.clone(),
            model.config_key(),
        )) {
            return Err(StbError::InvalidConfig(format!(
                "duplicate model identity {}:{} with identical config",
                model.provider_id, model.model_id
            ))
            .into());
        }

        let provider = provider_index
            .get(model.provider_id.as_str())
            .copied()
            .ok_or_else(|| {
                StbError::InvalidConfig(format!(
                    "model {} references unknown provider {}",
                    model.model_id, model.provider_id
                ))
            })?;

        if provider.endpoints.endpoint_for(&model.api_style).is_none() {
            return Err(StbError::InvalidConfig(format!(
                "provider {} does not define endpoint {} required by model {}",
                provider.provider_id,
                model.api_style.as_str(),
                model.model_id
            ))
            .into());
        }

        if model.timeout == 0 {
            return Err(StbError::InvalidConfig(format!(
                "model {} timeout must be greater than zero",
                model.model_id
            ))
            .into());
        }
    }

    let mut system_prompt_ids = BTreeSet::new();
    for prompt in &loaded.system_prompts {
        if prompt.id.trim().is_empty() {
            return Err(
                StbError::InvalidConfig("system prompt id must not be empty".to_string()).into(),
            );
        }

        if prompt.text.trim().is_empty() {
            return Err(StbError::InvalidConfig(format!(
                "system prompt {} must not be empty",
                prompt.id
            ))
            .into());
        }

        if !system_prompt_ids.insert(prompt.id.clone()) {
            return Err(StbError::InvalidConfig(format!(
                "duplicate system prompt id {}",
                prompt.id
            ))
            .into());
        }
    }

    let mut test_ids = BTreeSet::new();
    for test in &loaded.tests {
        if test.id.trim().is_empty() {
            return Err(StbError::InvalidConfig("test id must not be empty".to_string()).into());
        }

        if !test_ids.insert(test.id.clone()) {
            return Err(StbError::InvalidConfig(format!("duplicate test id {}", test.id)).into());
        }

        if loaded.system_prompts.is_empty() {
            return Err(StbError::InvalidConfig(
                "tests.json was provided but system_prompts.json is missing".to_string(),
            )
            .into());
        }

        if !system_prompt_ids.contains(&test.system_prompt) {
            return Err(StbError::InvalidConfig(format!(
                "test {} references unknown system prompt {}",
                test.id, test.system_prompt
            ))
            .into());
        }

        if test.input.is_empty() {
            return Err(StbError::InvalidConfig(format!(
                "test {} must define at least one input item",
                test.id
            ))
            .into());
        }
    }

    Ok(())
}

fn provider_index(loaded: &LoadedConfig) -> HashMap<&str, &ProviderConfig> {
    loaded
        .providers
        .iter()
        .map(|provider| (provider.provider_id.as_str(), provider))
        .collect()
}

fn read_json_file<T>(path: &Path) -> anyhow::Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", display_path(path)))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", display_path(path)))
}

fn read_optional_json_file<T>(path: &Path) -> anyhow::Result<Option<T>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path
        .try_exists()
        .with_context(|| format!("failed to access {}", display_path(path)))?
    {
        return Ok(None);
    }

    read_json_file(path).map(Some)
}

fn parse_json_str<T>(content: &str, source: &str) -> anyhow::Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(content).with_context(|| format!("failed to parse {source}"))
}

fn display_path(path: &Path) -> &str {
    path.to_str().unwrap_or("<non-utf8 path>")
}

const fn default_repeat() -> u32 {
    1
}

pub const fn default_streaming() -> bool {
    true
}

pub const fn default_model_timeout_seconds() -> u64 {
    DEFAULT_MODEL_TIMEOUT_SECONDS
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        ApiStyle, DEFAULT_MODEL_TIMEOUT_SECONDS, LoadedConfig, ModelConfig, ProviderConfig,
        ProviderEndpoints, TestCase, TestInput, load_loose_config, resolve_selected_models,
    };
    use tempfile::tempdir;

    fn base_provider() -> ProviderConfig {
        ProviderConfig {
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
        }
    }

    fn base_model() -> ModelConfig {
        ModelConfig {
            provider_id: "openrouter".to_string(),
            model_id: "z-ai/glm-5.1".to_string(),
            api_style: ApiStyle::OpenaiResponses,
            temperature: Some(0.0),
            max_output_tokens: Some(512),
            streaming: true,
            timeout: DEFAULT_MODEL_TIMEOUT_SECONDS,
            extra: Default::default(),
        }
    }

    #[test]
    fn loads_example_directory() {
        let loaded = load_loose_config(Path::new("example")).expect("example config should load");

        assert_eq!(loaded.providers.len(), 1);
        assert_eq!(loaded.models.len(), 4);
        assert_eq!(loaded.system_prompts.len(), 1);
        assert_eq!(loaded.tests.len(), 10);
    }

    #[test]
    fn rejects_duplicate_model_identity() {
        let loaded = LoadedConfig {
            input_dir: Path::new(".").to_path_buf(),
            providers: vec![base_provider()],
            models: vec![base_model(), base_model()],
            system_prompts: vec![],
            tests: vec![],
        };

        let error =
            super::validate_loaded_config(&loaded).expect_err("duplicate model should fail");
        assert!(error.to_string().contains("duplicate model identity"));
    }

    #[test]
    fn allows_same_provider_and_model_with_different_params() {
        let mut alternate_model = base_model();
        alternate_model.max_output_tokens = Some(1024);

        let loaded = LoadedConfig {
            input_dir: Path::new(".").to_path_buf(),
            providers: vec![base_provider()],
            models: vec![base_model(), alternate_model],
            system_prompts: vec![],
            tests: vec![],
        };

        super::validate_loaded_config(&loaded)
            .expect("models with different params should be distinct instances");
    }

    #[test]
    fn model_defaults_enable_streaming_and_timeout() {
        let model: ModelConfig = serde_json::from_str(
            r#"{
  "provider_id": "openrouter",
  "model_id": "z-ai/glm-5.1",
  "api_style": "openai_responses"
}"#,
        )
        .expect("model should deserialize");

        assert!(model.streaming);
        assert_eq!(model.timeout, DEFAULT_MODEL_TIMEOUT_SECONDS);
    }

    #[test]
    fn rejects_zero_model_timeout() {
        let mut model = base_model();
        model.timeout = 0;
        let loaded = LoadedConfig {
            input_dir: Path::new(".").to_path_buf(),
            providers: vec![base_provider()],
            models: vec![model],
            system_prompts: vec![],
            tests: vec![],
        };

        let error = super::validate_loaded_config(&loaded).expect_err("zero timeout should fail");
        assert!(
            error
                .to_string()
                .contains("timeout must be greater than zero")
        );
    }

    #[test]
    fn selects_multiple_instances_for_same_model_id() {
        let mut alternate_model = base_model();
        alternate_model.max_output_tokens = Some(1024);

        let loaded = LoadedConfig {
            input_dir: Path::new(".").to_path_buf(),
            providers: vec![base_provider()],
            models: vec![base_model(), alternate_model],
            system_prompts: vec![],
            tests: vec![],
        };

        let selected = resolve_selected_models(&loaded, Some("openrouter"), Some("z-ai/glm-5.1"))
            .expect("selection should include both instances");

        assert_eq!(selected.len(), 2);
        assert_ne!(
            selected[0].model.instance_id(),
            selected[1].model.instance_id()
        );
    }

    #[test]
    fn rejects_missing_endpoint_for_model_style() {
        let mut provider = base_provider();
        provider.endpoints.openai_responses = None;
        let loaded = LoadedConfig {
            input_dir: Path::new(".").to_path_buf(),
            providers: vec![provider],
            models: vec![base_model()],
            system_prompts: vec![],
            tests: vec![],
        };

        let error =
            super::validate_loaded_config(&loaded).expect_err("missing endpoint should fail");
        assert!(
            error
                .to_string()
                .contains("does not define endpoint openai_responses")
        );
    }

    #[test]
    fn resolves_selected_models_with_filters() {
        let loaded = LoadedConfig {
            input_dir: Path::new(".").to_path_buf(),
            providers: vec![base_provider()],
            models: vec![base_model()],
            system_prompts: vec![],
            tests: vec![],
        };

        let selected = resolve_selected_models(&loaded, Some("openrouter"), Some("z-ai/glm-5.1"))
            .expect("selection should work");

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].model.model_id, "z-ai/glm-5.1");
    }

    #[test]
    fn rejects_tests_without_matching_prompt() {
        let loaded = LoadedConfig {
            input_dir: Path::new(".").to_path_buf(),
            providers: vec![base_provider()],
            models: vec![base_model()],
            system_prompts: vec![],
            tests: vec![TestCase {
                id: "todo-001".to_string(),
                system_prompt: "missing".to_string(),
                input: vec![TestInput::Text {
                    text: "buy milk".to_string(),
                }],
                repeat: 1,
            }],
        };

        let error = super::validate_loaded_config(&loaded).expect_err("missing prompt should fail");
        assert!(error.to_string().contains("system_prompts.json is missing"));
    }

    #[test]
    fn reports_parse_errors_with_file_context() {
        let temp = tempdir().expect("temp dir should exist");
        std::fs::write(temp.path().join("providers.json"), "{not-json}")
            .expect("write should succeed");
        std::fs::write(temp.path().join("models.json"), "{\"models\":[]}")
            .expect("write should succeed");

        let error = load_loose_config(temp.path()).expect_err("bad json should fail");
        assert!(error.to_string().contains("providers.json"));
    }
}
