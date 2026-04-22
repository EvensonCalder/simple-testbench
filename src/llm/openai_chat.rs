use std::{thread, time::Duration};

use anyhow::{Context, anyhow};
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};

use crate::config::{ApiStyle, ModelConfig, ProviderConfig, SystemPrompt, TestCase};

use super::build_test_user_prompt;

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    max_retries: u8,
    retry_delays: Vec<Duration>,
}

impl RetryPolicy {
    pub fn from_retry_count(max_retries: u8) -> Self {
        let schedule = [
            Duration::from_secs(3),
            Duration::from_secs(10),
            Duration::from_secs(30),
        ];
        let retry_delays = schedule
            .into_iter()
            .take(max_retries as usize)
            .collect::<Vec<_>>();

        Self {
            max_retries,
            retry_delays,
        }
    }

    pub fn with_delays(max_retries: u8, retry_delays: Vec<Duration>) -> Self {
        Self {
            max_retries,
            retry_delays,
        }
    }

    pub fn max_retries(&self) -> u8 {
        self.max_retries
    }

    pub(crate) fn retry_delay(&self, retry_index: usize) -> Duration {
        self.retry_delays
            .get(retry_index)
            .copied()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct ChatExecution {
    pub output_text: String,
    pub attempts: u8,
}

pub fn execute_openai_chat_completion(
    provider: &ProviderConfig,
    model: &ModelConfig,
    system_prompt: &SystemPrompt,
    test_case: &TestCase,
    retry_policy: &RetryPolicy,
    verbose: bool,
) -> anyhow::Result<ChatExecution> {
    let user_prompt = build_test_user_prompt(test_case)?;
    execute_openai_chat_completion_prompt(
        provider,
        model,
        &system_prompt.text,
        &user_prompt,
        retry_policy,
        verbose,
        &format!("test={}", test_case.id),
    )
}

pub fn execute_openai_chat_completion_prompt(
    provider: &ProviderConfig,
    model: &ModelConfig,
    system_prompt_text: &str,
    user_prompt: &str,
    retry_policy: &RetryPolicy,
    verbose: bool,
    request_label: &str,
) -> anyhow::Result<ChatExecution> {
    if model.api_style != ApiStyle::OpenaiChatCompletions {
        return Err(anyhow!(
            "model {} is configured as {}, not openai_chat_completions",
            model.model_id,
            model.api_style.as_str()
        ));
    }

    let endpoint = provider
        .endpoints
        .endpoint_for(&ApiStyle::OpenaiChatCompletions)
        .ok_or_else(|| {
            anyhow!(
                "provider {} does not support openai_chat_completions",
                provider.provider_id
            )
        })?;
    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));
    let api_key = resolve_api_key(provider)?;
    let request_body = build_chat_request(model, system_prompt_text, user_prompt);
    let client = Client::builder()
        .build()
        .context("failed to build HTTP client")?;

    for attempt_index in 0..=retry_policy.max_retries() {
        let attempt_number = attempt_index + 1;

        if verbose {
            println!(
                "requesting {}/{} {} attempt={}",
                provider.provider_id, model.model_id, request_label, attempt_number
            );
        }

        match send_request(&client, &url, &api_key, &request_body) {
            Ok(output_text) => {
                return Ok(ChatExecution {
                    output_text,
                    attempts: attempt_number,
                });
            }
            Err(error) if attempt_index < retry_policy.max_retries() => {
                if verbose {
                    println!(
                        "request failed for {}/{} {} attempt={} error={error}",
                        provider.provider_id, model.model_id, request_label, attempt_number
                    );
                }

                thread::sleep(retry_policy.retry_delay(attempt_index as usize));
            }
            Err(error) => {
                return Err(error.context(format!(
                    "chat completion failed for {}/{} {} after {} attempts",
                    provider.provider_id, model.model_id, request_label, attempt_number
                )));
            }
        }
    }

    unreachable!("retry loop should always return")
}

fn send_request(
    client: &Client,
    url: &str,
    api_key: &str,
    request_body: &Value,
) -> anyhow::Result<String> {
    let response = client
        .post(url)
        .bearer_auth(api_key)
        .json(request_body)
        .send()
        .with_context(|| format!("failed to send request to {url}"))?;

    let status = response.status();
    let response_text = response.text().context("failed to read response body")?;

    if !status.is_success() {
        return Err(anyhow!(
            "request returned status {} with body {}",
            status,
            response_text
        ));
    }

    extract_output_text(&response_text)
}

fn extract_output_text(response_text: &str) -> anyhow::Result<String> {
    let payload = serde_json::from_str::<Value>(response_text)
        .context("failed to parse chat completion response JSON")?;

    payload
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("response did not contain choices[0].message.content"))
}

fn build_chat_request(model: &ModelConfig, system_prompt_text: &str, user_prompt: &str) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(model.model_id.clone()));
    body.insert(
        "messages".to_string(),
        json!([
            {
                "role": "system",
                "content": system_prompt_text,
            },
            {
                "role": "user",
                "content": user_prompt,
            }
        ]),
    );

    if let Some(temperature) = model.temperature {
        body.insert("temperature".to_string(), json!(temperature));
    }

    if let Some(max_output_tokens) = model.max_output_tokens {
        body.insert("max_tokens".to_string(), json!(max_output_tokens));
    }

    for (key, value) in &model.extra {
        body.insert(key.clone(), value.clone());
    }

    Value::Object(body)
}

fn resolve_api_key(provider: &ProviderConfig) -> anyhow::Result<String> {
    if let Some(key) = &provider.key {
        return Ok(key.clone());
    }

    let env_key_name = provider.env_key.as_deref().ok_or_else(|| {
        anyhow!(
            "provider {} is missing key and env_key",
            provider.provider_id
        )
    })?;

    std::env::var(env_key_name)
        .with_context(|| format!("failed to read API key from environment variable {env_key_name}"))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use crate::config::{ApiStyle, ModelConfig, ProviderConfig, ProviderEndpoints};

    use super::{RetryPolicy, build_chat_request, extract_output_text};

    fn model() -> ModelConfig {
        ModelConfig {
            provider_id: "mock".to_string(),
            model_id: "chat-model".to_string(),
            api_style: ApiStyle::OpenaiChatCompletions,
            temperature: Some(0.0),
            max_output_tokens: Some(256),
            extra: Default::default(),
        }
    }

    #[test]
    fn builds_expected_chat_request() {
        let request = build_chat_request(&model(), "Return JSON", "buy milk at the store");

        assert_eq!(request["model"], json!("chat-model"));
        assert_eq!(request["temperature"], json!(0.0));
        assert_eq!(request["max_tokens"], json!(256));
        assert_eq!(request["messages"][0]["role"], json!("system"));
        assert_eq!(
            request["messages"][1]["content"],
            json!("buy milk at the store")
        );
    }

    #[test]
    fn extracts_output_text_from_response() {
        let output = extract_output_text(
            r#"{"choices":[{"message":{"content":"{\"todo\":\"buy milk\"}"}}]}"#,
        )
        .expect("output should parse");

        assert_eq!(output, r#"{"todo":"buy milk"}"#);
    }

    #[test]
    fn retry_policy_uses_expected_delays() {
        let policy = RetryPolicy::from_retry_count(2);
        assert_eq!(policy.max_retries(), 2);

        let custom = RetryPolicy::with_delays(2, vec![Duration::ZERO, Duration::from_millis(1)]);
        assert_eq!(custom.retry_delay(0), Duration::ZERO);
        assert_eq!(custom.retry_delay(1), Duration::from_millis(1));
    }

    #[test]
    fn resolves_provider_key_from_plaintext() {
        let provider = ProviderConfig {
            provider_id: "mock".to_string(),
            key: Some("plain-key".to_string()),
            env_key: None,
            concurrency: 1,
            rpm: 10,
            endpoints: ProviderEndpoints {
                openai_chat_completions: Some("http://example.test".to_string()),
                openai_responses: None,
                anthropic_messages: None,
            },
        };

        let key = super::resolve_api_key(&provider).expect("key should resolve");
        assert_eq!(key, "plain-key");
    }
}
