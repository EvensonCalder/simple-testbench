use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow};
use reqwest::Client;
use serde_json::{Map, Value, json};

use crate::config::{ApiStyle, ModelConfig, ProviderConfig};

use super::RetryPolicy;

#[derive(Debug, Clone)]
pub struct MessagesExecution {
    pub output_text: String,
    pub attempts: u8,
    pub elapsed_ms: u64,
}

pub fn execute_anthropic_messages_prompt(
    provider: &ProviderConfig,
    model: &ModelConfig,
    system_prompt_text: &str,
    user_prompt: &str,
    retry_policy: &RetryPolicy,
    verbose: bool,
    request_label: &str,
) -> anyhow::Result<MessagesExecution> {
    if model.api_style != ApiStyle::AnthropicMessages {
        return Err(anyhow!(
            "model {} is configured as {}, not anthropic_messages",
            model.model_id,
            model.api_style.as_str()
        ));
    }

    let endpoint = provider
        .endpoints
        .endpoint_for(&ApiStyle::AnthropicMessages)
        .ok_or_else(|| {
            anyhow!(
                "provider {} does not support anthropic_messages",
                provider.provider_id
            )
        })?;
    let url = messages_url(endpoint);
    let api_key = resolve_api_key(provider)?;
    let request_body = build_messages_request(model, system_prompt_text, user_prompt);
    let client = Client::builder()
        .build()
        .context("failed to build HTTP client")?;
    let idle_timeout = Duration::from_secs(model.timeout);
    let mut elapsed_ms = 0u64;

    for attempt_index in 0..=retry_policy.max_retries() {
        let attempt_number = attempt_index + 1;

        if verbose {
            println!(
                "requesting {}/{} {} attempt={}",
                provider.provider_id, model.model_id, request_label, attempt_number
            );
        }

        let started = Instant::now();
        match super::streaming::block_on_http(send_request(
            &client,
            &url,
            &api_key,
            &request_body,
            model.streaming,
            idle_timeout,
        )) {
            Ok(output_text) => {
                elapsed_ms += started.elapsed().as_millis() as u64;
                return Ok(MessagesExecution {
                    output_text,
                    attempts: attempt_number,
                    elapsed_ms,
                });
            }
            Err(error) if attempt_index < retry_policy.max_retries() => {
                elapsed_ms += started.elapsed().as_millis() as u64;
                if verbose {
                    println!(
                        "request failed for {}/{} {} attempt={} error={error}",
                        provider.provider_id, model.model_id, request_label, attempt_number
                    );
                }

                thread::sleep(retry_delay(retry_policy, attempt_index as usize));
            }
            Err(error) => {
                return Err(error.context(format!(
                    "anthropic messages request failed for {}/{} {} after {} attempts",
                    provider.provider_id, model.model_id, request_label, attempt_number
                )));
            }
        }
    }

    unreachable!("retry loop should always return")
}

async fn send_request(
    client: &Client,
    url: &str,
    api_key: &str,
    request_body: &Value,
    streaming: bool,
    idle_timeout: Duration,
) -> anyhow::Result<String> {
    let response = tokio::time::timeout(
        idle_timeout,
        client
            .post(url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .bearer_auth(api_key)
            .json(request_body)
            .send(),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "request to {url} timed out after {} seconds",
            idle_timeout.as_secs()
        )
    })?
    .with_context(|| format!("failed to send request to {url}"))?;

    let status = response.status();

    if !status.is_success() {
        let response_text = tokio::time::timeout(idle_timeout, response.text())
            .await
            .map_err(|_| {
                anyhow!(
                    "response body timed out after {} seconds",
                    idle_timeout.as_secs()
                )
            })?
            .context("failed to read response body")?;
        return Err(anyhow!(
            "request returned status {} with body {}",
            status,
            response_text
        ));
    }

    if streaming {
        return extract_streaming_output_text(response, idle_timeout).await;
    }

    let response_text = tokio::time::timeout(idle_timeout, response.text())
        .await
        .map_err(|_| {
            anyhow!(
                "response body timed out after {} seconds",
                idle_timeout.as_secs()
            )
        })?
        .context("failed to read response body")?;
    extract_output_text(&response_text)
}

fn build_messages_request(
    model: &ModelConfig,
    system_prompt_text: &str,
    user_prompt: &str,
) -> Value {
    let mut body = Map::new();
    body.insert("model".to_string(), Value::String(model.model_id.clone()));
    body.insert("stream".to_string(), json!(model.streaming));
    body.insert(
        "system".to_string(),
        Value::String(system_prompt_text.to_string()),
    );
    body.insert(
        "messages".to_string(),
        json!([
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": user_prompt,
                    }
                ]
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

fn extract_output_text(response_text: &str) -> anyhow::Result<String> {
    let payload = serde_json::from_str::<Value>(response_text)
        .context("failed to parse anthropic messages response JSON")?;
    let content = payload
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("response did not contain content array"))?;

    let mut parts = Vec::new();
    for block in content {
        if block.get("type").and_then(Value::as_str) == Some("text")
            && let Some(text) = block.get("text").and_then(Value::as_str)
        {
            parts.push(text.to_string());
        }
    }

    if parts.is_empty() {
        return Err(anyhow!("response did not contain any final text blocks"));
    }

    Ok(parts.join("\n\n"))
}

async fn extract_streaming_output_text(
    response: reqwest::Response,
    idle_timeout: Duration,
) -> anyhow::Result<String> {
    let mut output = String::new();

    super::streaming::read_sse_response(response, idle_timeout, |data| {
        if data.trim() == "[DONE]" {
            return Ok(true);
        }

        let payload = serde_json::from_str::<Value>(data)
            .context("failed to parse anthropic messages stream event JSON")?;

        if payload.get("type").and_then(Value::as_str) == Some("error") {
            return Err(anyhow!("stream returned error {payload}"));
        }

        match payload.get("type").and_then(Value::as_str) {
            Some("content_block_delta") => {
                if let Some(text) = payload
                    .get("delta")
                    .and_then(|delta| delta.get("text"))
                    .and_then(Value::as_str)
                {
                    output.push_str(text);
                }
            }
            Some("message_stop") => return Ok(true),
            _ => {}
        }

        Ok(false)
    })
    .await?;

    if output.is_empty() {
        return Err(anyhow!(
            "stream response did not contain content_block_delta text"
        ));
    }

    Ok(output)
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

fn messages_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim_end_matches('/');
    if trimmed.ends_with("/messages") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/messages")
    }
}

fn retry_delay(retry_policy: &RetryPolicy, retry_index: usize) -> Duration {
    retry_policy.retry_delay(retry_index)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::config::{ApiStyle, ModelConfig};

    use super::{build_messages_request, extract_output_text};

    fn model() -> ModelConfig {
        ModelConfig {
            provider_id: "mock".to_string(),
            model_id: "messages-model".to_string(),
            api_style: ApiStyle::AnthropicMessages,
            temperature: Some(0.0),
            max_output_tokens: Some(256),
            streaming: true,
            timeout: 300,
            extra: Default::default(),
        }
    }

    #[test]
    fn builds_expected_messages_request() {
        let request = build_messages_request(&model(), "Return JSON", "buy milk");

        assert_eq!(request["model"], json!("messages-model"));
        assert_eq!(request["stream"], json!(true));
        assert_eq!(request["system"], json!("Return JSON"));
        assert_eq!(request["temperature"], json!(0.0));
        assert_eq!(request["max_tokens"], json!(256));
        assert_eq!(
            request["messages"][0]["content"][0]["text"],
            json!("buy milk")
        );
    }

    #[test]
    fn extracts_text_and_discards_thinking_blocks() {
        let output = extract_output_text(
            r#"{"content":[{"type":"thinking","thinking":"hidden"},{"type":"text","text":"final answer"}]}"#,
        )
        .expect("output should parse");

        assert_eq!(output, "final answer");
    }
}
