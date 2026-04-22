//! LLM request transport and normalization.

use anyhow::anyhow;

use crate::config::{ApiStyle, ModelConfig, ProviderConfig, TestCase, TestInput};

pub mod anthropic_messages;
pub mod openai_chat;
pub mod openai_responses;

pub use openai_chat::RetryPolicy;

#[derive(Debug, Clone)]
pub struct ModelExecution {
    pub output_text: String,
    pub attempts: u8,
}

pub fn execute_model_request(
    provider: &ProviderConfig,
    model: &ModelConfig,
    system_prompt_text: &str,
    user_prompt: &str,
    retry_policy: &RetryPolicy,
    verbose: bool,
    request_label: &str,
) -> anyhow::Result<ModelExecution> {
    match model.api_style {
        ApiStyle::OpenaiChatCompletions => openai_chat::execute_openai_chat_completion_prompt(
            provider,
            model,
            system_prompt_text,
            user_prompt,
            retry_policy,
            verbose,
            request_label,
        )
        .map(|execution| ModelExecution {
            output_text: execution.output_text,
            attempts: execution.attempts,
        }),
        ApiStyle::OpenaiResponses => openai_responses::execute_openai_responses_prompt(
            provider,
            model,
            system_prompt_text,
            user_prompt,
            retry_policy,
            verbose,
            request_label,
        )
        .map(|execution| ModelExecution {
            output_text: execution.output_text,
            attempts: execution.attempts,
        }),
        ApiStyle::AnthropicMessages => anthropic_messages::execute_anthropic_messages_prompt(
            provider,
            model,
            system_prompt_text,
            user_prompt,
            retry_policy,
            verbose,
            request_label,
        )
        .map(|execution| ModelExecution {
            output_text: execution.output_text,
            attempts: execution.attempts,
        }),
    }
}

pub fn build_test_user_prompt(test_case: &TestCase) -> anyhow::Result<String> {
    let parts = test_case
        .input
        .iter()
        .map(|item| match item {
            TestInput::Text { text } => text.as_str(),
        })
        .collect::<Vec<_>>();

    if parts.is_empty() {
        return Err(anyhow!(
            "test {} does not contain any input text",
            test_case.id
        ));
    }

    Ok(parts.join("\n\n"))
}
