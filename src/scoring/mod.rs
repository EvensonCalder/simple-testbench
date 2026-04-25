//! Post-processing and scoring pipelines.

use std::{collections::BTreeSet, fs, path::Path};

use anyhow::{Context, anyhow};
use mlua::{Function, Lua, Table};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    archive,
    config::{
        ApiStyle, DEFAULT_MODEL_TIMEOUT_SECONDS, LoadedConfig, ModelConfig, TestCase,
        default_streaming,
    },
    error::StbError,
    llm::{self, RetryPolicy},
    output::{ScoreResult, ScoreStatus},
};

const SCORING_FILE: &str = "scoring.json";
const POST_PROCESS_FILE: &str = "post_process.lua";

#[derive(Debug, Clone, Default)]
pub struct LoadedScoringConfig {
    pub post_process: Option<String>,
    pub scorers: Vec<LoadedScorer>,
}

impl LoadedScoringConfig {
    pub fn scorer_names(&self) -> Vec<&str> {
        self.scorers
            .iter()
            .map(|scorer| scorer.name.as_str())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct LoadedScorer {
    pub name: String,
    pub kind: LoadedScorerKind,
}

#[derive(Debug, Clone)]
pub enum LoadedScorerKind {
    Lua { script: String },
    Ai { config: AiScoringConfig },
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiScoringConfig {
    pub provider_id: String,
    pub model_id: String,
    pub api_style: ApiStyle,
    pub temperature: Option<f64>,
    pub max_output_tokens: Option<u32>,
    #[serde(default = "default_streaming")]
    pub streaming: bool,
    #[serde(default = "default_ai_timeout")]
    pub timeout: u64,
    pub system_prompt: String,
    #[serde(flatten, default)]
    pub extra: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostProcessOutcome {
    pub output: String,
    pub retry: bool,
    pub max_retry: u32,
}

#[derive(Debug, Deserialize)]
struct ScoringFile {
    scoring: Vec<ScoringItem>,
}

#[derive(Debug, Deserialize)]
struct ScoringItem {
    name: String,
    kind: ScoringKind,
    file: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ScoringKind {
    Lua,
    Ai,
}

pub fn load_scoring_config(
    input_dir: &Path,
    score_archive: Option<&Path>,
) -> anyhow::Result<LoadedScoringConfig> {
    if let Some(path) = score_archive {
        let bundle = archive::load_scoring_bundle(path)?;
        let scoring = serde_json::from_str::<ScoringFile>(&bundle.scoring_json)
            .with_context(|| format!("failed to parse {SCORING_FILE}"))?;
        return build_scoring_config(scoring, bundle.post_process_lua, |file| {
            bundle
                .files
                .get(file)
                .cloned()
                .ok_or_else(|| anyhow!("missing {file} in scoring archive"))
        });
    }

    let scoring_path = input_dir.join(SCORING_FILE);
    let post_process_path = input_dir.join(POST_PROCESS_FILE);
    let post_process = read_optional_text_file(&post_process_path)?;

    if !scoring_path
        .try_exists()
        .with_context(|| format!("failed to access {}", display_path(&scoring_path)))?
    {
        return Ok(LoadedScoringConfig {
            post_process,
            scorers: Vec::new(),
        });
    }

    let scoring = read_json_file::<ScoringFile>(&scoring_path)?;
    build_scoring_config(scoring, post_process, |file| {
        let path = input_dir.join(file);
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", display_path(&path)))
    })
}

pub fn apply_post_process(script: &str, raw_output: &str) -> anyhow::Result<PostProcessOutcome> {
    let lua = Lua::new();
    let function = load_lua_function(&lua, script, "post_process.lua")?;
    let table: Table = function
        .call(raw_output)
        .map_err(|error| anyhow!("post-process function failed: {error}"))?;
    let output = table
        .get::<Option<String>>("output")
        .map_err(|error| anyhow!("post-process output field is invalid: {error}"))?
        .unwrap_or_else(|| raw_output.to_string());
    let retry = table
        .get::<Option<bool>>("retry")
        .map_err(|error| anyhow!("post-process retry field is invalid: {error}"))?
        .unwrap_or(false);
    let max_retry = table
        .get::<Option<u32>>("max_retry")
        .map_err(|error| anyhow!("post-process max_retry field is invalid: {error}"))?
        .unwrap_or(0);

    Ok(PostProcessOutcome {
        output,
        retry,
        max_retry,
    })
}

pub fn score_processed_output(
    scoring: &LoadedScoringConfig,
    loaded: &LoadedConfig,
    test_case: &TestCase,
    processed_output: &str,
    retry_policy: &RetryPolicy,
    verbose: bool,
) -> Vec<ScoreResult> {
    scoring
        .scorers
        .iter()
        .map(|scorer| match &scorer.kind {
            LoadedScorerKind::Lua { script } => {
                evaluate_lua_scorer(&scorer.name, script, processed_output)
            }
            LoadedScorerKind::Ai { config } => evaluate_ai_scorer(
                &scorer.name,
                config,
                loaded,
                test_case,
                processed_output,
                retry_policy,
                verbose,
            ),
        })
        .collect()
}

fn evaluate_lua_scorer(name: &str, script: &str, processed_output: &str) -> ScoreResult {
    match evaluate_lua_score(script, processed_output) {
        Ok(score) => ScoreResult {
            name: name.to_string(),
            kind: "lua".to_string(),
            status: ScoreStatus::Success,
            score: Some(score),
            details: None,
            error: None,
        },
        Err(error) => ScoreResult {
            name: name.to_string(),
            kind: "lua".to_string(),
            status: ScoreStatus::Failed,
            score: None,
            details: None,
            error: Some(error.to_string()),
        },
    }
}

fn evaluate_ai_scorer(
    name: &str,
    config: &AiScoringConfig,
    loaded: &LoadedConfig,
    test_case: &TestCase,
    processed_output: &str,
    retry_policy: &RetryPolicy,
    verbose: bool,
) -> ScoreResult {
    let provider = match loaded
        .providers
        .iter()
        .find(|provider| provider.provider_id == config.provider_id)
    {
        Some(provider) => provider,
        None => {
            return ScoreResult {
                name: name.to_string(),
                kind: "ai".to_string(),
                status: ScoreStatus::Failed,
                score: None,
                details: None,
                error: Some(format!(
                    "AI scorer references unknown provider {}",
                    config.provider_id
                )),
            };
        }
    };

    let judge_model = ModelConfig {
        provider_id: config.provider_id.clone(),
        model_id: config.model_id.clone(),
        api_style: config.api_style.clone(),
        temperature: config.temperature,
        max_output_tokens: config.max_output_tokens,
        streaming: config.streaming,
        timeout: config.timeout,
        extra: config.extra.clone(),
    };
    let benchmark_input = match llm::build_test_user_prompt(test_case) {
        Ok(text) => text,
        Err(error) => {
            return ScoreResult {
                name: name.to_string(),
                kind: "ai".to_string(),
                status: ScoreStatus::Failed,
                score: None,
                details: None,
                error: Some(error.to_string()),
            };
        }
    };
    let judge_prompt = build_ai_judge_prompt(&benchmark_input, processed_output);

    match llm::execute_model_request(
        provider,
        &judge_model,
        &config.system_prompt,
        &judge_prompt,
        retry_policy,
        verbose,
        &format!("score={} test={}", name, test_case.id),
    ) {
        Ok(execution) => match parse_ai_score_response(&execution.output_text) {
            Ok((score, reason)) => ScoreResult {
                name: name.to_string(),
                kind: "ai".to_string(),
                status: ScoreStatus::Success,
                score: Some(score),
                details: reason,
                error: None,
            },
            Err(error) => ScoreResult {
                name: name.to_string(),
                kind: "ai".to_string(),
                status: ScoreStatus::Failed,
                score: None,
                details: Some(execution.output_text),
                error: Some(error.to_string()),
            },
        },
        Err(error) => ScoreResult {
            name: name.to_string(),
            kind: "ai".to_string(),
            status: ScoreStatus::Failed,
            score: None,
            details: None,
            error: Some(error.to_string()),
        },
    }
}

const fn default_ai_timeout() -> u64 {
    DEFAULT_MODEL_TIMEOUT_SECONDS
}

fn build_ai_judge_prompt(benchmark_input: &str, processed_output: &str) -> String {
    format!("Benchmark input:\n{benchmark_input}\n\nCandidate output:\n{processed_output}\n")
}

fn parse_ai_score_response(response_text: &str) -> anyhow::Result<(u8, Option<String>)> {
    let normalized = strip_json_code_fence(response_text);
    let payload = serde_json::from_str::<Value>(&normalized)
        .context("AI scorer response was not valid JSON")?;
    let score_value = payload
        .get("score")
        .ok_or_else(|| anyhow!("AI scorer response did not include a score field"))?;
    let score = match score_value {
        Value::Number(number) => number
            .as_u64()
            .ok_or_else(|| anyhow!("AI scorer score must be a non-negative integer"))?,
        Value::String(text) => text
            .parse::<u64>()
            .map_err(|_| anyhow!("AI scorer score string must contain an integer"))?,
        _ => {
            return Err(anyhow!(
                "AI scorer score must be an integer or integer string"
            ));
        }
    };

    if score > 100 {
        return Err(anyhow!("AI scorer score must be between 0 and 100"));
    }

    let reason = payload
        .get("reason")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);

    Ok((score as u8, reason))
}

fn strip_json_code_fence(text: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }

    let mut lines = trimmed.lines();
    let Some(first_line) = lines.next() else {
        return trimmed.to_string();
    };

    if !first_line.starts_with("```") {
        return trimmed.to_string();
    }

    let body = lines.collect::<Vec<_>>().join("\n");
    let body = body.trim_end();
    let Some(stripped) = body.strip_suffix("```") else {
        return trimmed.to_string();
    };

    stripped.trim().to_string()
}

fn build_scoring_config<F>(
    scoring: ScoringFile,
    post_process: Option<String>,
    mut load_file: F,
) -> anyhow::Result<LoadedScoringConfig>
where
    F: FnMut(&str) -> anyhow::Result<String>,
{
    let mut names = BTreeSet::new();
    let mut scorers = Vec::with_capacity(scoring.scoring.len());

    for item in scoring.scoring {
        if item.name.trim().is_empty() {
            return Err(StbError::InvalidConfig("score name must not be empty".to_string()).into());
        }

        if !names.insert(item.name.clone()) {
            return Err(
                StbError::InvalidConfig(format!("duplicate score name {}", item.name)).into(),
            );
        }

        let content = load_file(&item.file)?;
        let kind = match item.kind {
            ScoringKind::Lua => LoadedScorerKind::Lua { script: content },
            ScoringKind::Ai => {
                let config: AiScoringConfig = serde_json::from_str(&content)
                    .with_context(|| format!("failed to parse AI scorer config {}", item.file))?;
                if config.timeout == 0 {
                    return Err(StbError::InvalidConfig(format!(
                        "AI scorer {} timeout must be greater than zero",
                        item.name
                    ))
                    .into());
                }

                LoadedScorerKind::Ai { config }
            }
        };

        scorers.push(LoadedScorer {
            name: item.name,
            kind,
        });
    }

    Ok(LoadedScoringConfig {
        post_process,
        scorers,
    })
}

fn evaluate_lua_score(script: &str, processed_output: &str) -> anyhow::Result<u8> {
    let lua = Lua::new();
    let function = load_lua_function(&lua, script, "lua scorer")?;
    let score = function
        .call::<i64>(processed_output)
        .map_err(|error| anyhow!("lua scorer function failed: {error}"))?;

    if !(0..=100).contains(&score) {
        return Err(anyhow!(
            "lua scorer returned {score}, expected an integer from 0 to 100"
        ));
    }

    u8::try_from(score).context("lua scorer returned a score outside u8 range")
}

fn load_lua_function(lua: &Lua, script: &str, name: &str) -> anyhow::Result<Function> {
    lua.load(script)
        .set_name(name)
        .eval::<Function>()
        .map_err(|error| anyhow!("{name} must return a function: {error}"))
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

fn read_optional_text_file(path: &Path) -> anyhow::Result<Option<String>> {
    if !path
        .try_exists()
        .with_context(|| format!("failed to access {}", display_path(path)))?
    {
        return Ok(None);
    }

    fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", display_path(path)))
        .map(Some)
}

fn display_path(path: &Path) -> &str {
    path.to_str().unwrap_or("<non-utf8 path>")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        LoadedScorerKind, apply_post_process, load_scoring_config, parse_ai_score_response,
        score_processed_output,
    };
    use crate::config::{
        LoadedConfig, ProviderConfig, ProviderEndpoints, SystemPrompt, TestCase, TestInput,
    };
    use crate::llm::RetryPolicy;

    #[test]
    fn loads_loose_scoring_assets() {
        let temp = tempdir().expect("temp dir should exist");
        let root = temp.path();

        fs::write(
            root.join("scoring.json"),
            r#"{
  "scoring": [
    {
      "name": "shape",
      "kind": "lua",
      "file": "score_shape.lua"
    },
    {
      "name": "judge",
      "kind": "ai",
      "file": "judge.json"
    }
  ]
}"#,
        )
        .expect("scoring should be written");
        fs::write(
            root.join("post_process.lua"),
            "return function(raw_output) return { output = raw_output } end",
        )
        .expect("post-process should be written");
        fs::write(
            root.join("score_shape.lua"),
            "return function(output) return 100 end",
        )
        .expect("lua scorer should be written");
        fs::write(
            root.join("judge.json"),
            r#"{
  "provider_id": "mock",
  "model_id": "judge-model",
  "api_style": "openai_responses",
  "temperature": 0,
  "system_prompt": "Score it"
}"#,
        )
        .expect("ai config should be written");

        let loaded = load_scoring_config(root, None).expect("scoring config should load");

        assert!(loaded.post_process.is_some());
        assert_eq!(loaded.scorers.len(), 2);
        assert!(matches!(
            loaded.scorers[0].kind,
            LoadedScorerKind::Lua { .. }
        ));
        match &loaded.scorers[1].kind {
            LoadedScorerKind::Ai { config } => {
                assert!(config.streaming);
                assert_eq!(config.timeout, crate::config::DEFAULT_MODEL_TIMEOUT_SECONDS);
            }
            LoadedScorerKind::Lua { .. } => panic!("expected AI scorer"),
        }
    }

    #[test]
    fn applies_post_process_contract() {
        let outcome = apply_post_process(
            r#"return function(raw_output)
  return {
    output = tostring(raw_output):gsub("^%s+", ""):gsub("%s+$", ""),
    retry = true,
    max_retry = 2,
  }
end"#,
            "  hello  ",
        )
        .expect("post-process should succeed");

        assert_eq!(outcome.output, "hello");
        assert!(outcome.retry);
        assert_eq!(outcome.max_retry, 2);
    }

    #[test]
    fn scores_output_with_lua_scorer() {
        let temp = tempdir().expect("temp dir should exist");
        let root = temp.path();

        fs::write(
            root.join("scoring.json"),
            r#"{
  "scoring": [
    {
      "name": "shape",
      "kind": "lua",
      "file": "score_shape.lua"
    }
  ]
}"#,
        )
        .expect("scoring should be written");
        fs::write(
            root.join("score_shape.lua"),
            "return function(output) if output == 'ok' then return 100 end return 0 end",
        )
        .expect("lua scorer should be written");

        let loaded_scoring = load_scoring_config(root, None).expect("scoring config should load");
        let loaded_config = LoadedConfig {
            input_dir: root.to_path_buf(),
            providers: vec![ProviderConfig {
                provider_id: "mock".to_string(),
                key: Some("key".to_string()),
                env_key: None,
                concurrency: 1,
                rpm: 10,
                endpoints: ProviderEndpoints {
                    openai_chat_completions: Some("http://example.test".to_string()),
                    openai_responses: Some("http://example.test/responses".to_string()),
                    anthropic_messages: Some("http://example.test/messages".to_string()),
                },
            }],
            models: vec![],
            system_prompts: vec![SystemPrompt {
                id: "prompt".to_string(),
                text: "Return text".to_string(),
            }],
            tests: vec![],
        };
        let test_case = TestCase {
            id: "test-1".to_string(),
            system_prompt: "prompt".to_string(),
            input: vec![TestInput::Text {
                text: "hello".to_string(),
            }],
            repeat: 1,
        };

        let results = score_processed_output(
            &loaded_scoring,
            &loaded_config,
            &test_case,
            "ok",
            &RetryPolicy::with_delays(0, vec![]),
            false,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].score, Some(100));
    }

    #[test]
    fn parses_ai_score_output() {
        let (score, reason) = parse_ai_score_response(r#"{"score":91,"reason":"good"}"#)
            .expect("ai score should parse");

        assert_eq!(score, 91);
        assert_eq!(reason.as_deref(), Some("good"));
    }

    #[test]
    fn parses_ai_score_output_inside_code_fence() {
        let (score, reason) = parse_ai_score_response(
            "```json\n{\n  \"score\": 88,\n  \"reason\": \"solid\"\n}\n```",
        )
        .expect("fenced ai score should parse");

        assert_eq!(score, 88);
        assert_eq!(reason.as_deref(), Some("solid"));
    }
}
