# Architecture

## Overview

STB is a benchmarking-first CLI, not a general-purpose chat client.

Its execution pipeline is:

1. Load provider and model definitions.
2. Load tests from loose files or `.stbt`.
3. Build an execution plan.
4. Run model requests with retries and rate limits.
5. Normalize model outputs and discard reasoning or thinking content.
6. Optionally post-process outputs through Lua.
7. Score each output through Lua or AI judges.
8. Persist intermediate state to `output.json`.
9. Aggregate scores into JSON, CSV, and terminal-table outputs.

## Top-Level Modules

### `src/cli.rs`

Owns Clap definitions and argument validation.

### `src/app.rs`

Bridges CLI commands to library workflows.

### `src/config/`

Will load and validate:

- `providers.json`
- `models.json`
- `system_prompts.json`
- `tests.json`
- `scoring.json`
- AI scoring config files

### `src/llm/`

Will own provider transport logic, retries, throttling, and response normalization.

Planned request adapters:

- OpenAI Chat Completions
- OpenAI Responses
- Anthropic Messages

### `src/runner/`

Will own planning, scheduling, resume behavior, and execution state transitions.

### `src/scoring/`

Will own Lua post-processing, Lua scoring, and AI judge orchestration.

### `src/archive/`

Will own `.stbt` and `.stbs` pack and unpack support.

### `src/output/`

Will own `output.json`, `results.json`, CSV writers, and terminal rendering.

## Key Data Decisions

### Provider identity

A runnable model instance is identified by:

- `provider_id`
- `model_id`

This avoids collisions across providers that expose the same model name.

### Multi-endpoint providers

One provider definition may support multiple API styles.

Example:

```json
{
  "provider_id": "openrouter",
  "env_key": "OPENROUTER_API_KEY",
  "concurrency": 4,
  "rpm": 60,
  "endpoints": {
    "openai_chat_completions": "https://openrouter.ai/api/v1",
    "openai_responses": "https://openrouter.ai/api/v1/responses",
    "anthropic_messages": "https://openrouter.ai/api/v1/messages"
  }
}
```

### Output normalization

Every request adapter must normalize provider-specific responses into one internal output model.

Required behavior:

- keep only the user-visible final answer
- discard reasoning and thinking content
- support streaming by default and non-streaming when a model config sets `streaming` to `false`
- apply each model's read `timeout` so an idle provider connection fails and enters the retry policy
- preserve raw metadata for verbose inspection and resume state

### Retry policy

Request retries default to three attempts with delays:

- 3 seconds
- 10 seconds
- 30 seconds

If all retries fail, STB disables that provider-model instance for the remainder of the run.

### Post-process retry policy

`post_process.lua` may request a retry and optionally set `max_retry`.

STB tracks the retry count and, after the maximum is reached, continues with the last available result.

## Why This Structure

This layout keeps the risky moving parts isolated:

- transport complexity stays in `llm/`
- workflow complexity stays in `runner/`
- script execution stays in `scoring/`
- serialization and reporting stay in `output/`

That separation is the main guardrail for testability and future extension.

## External Library Guidance

External Rust LLM libraries are references, not the planned core transport layer.

STB should keep direct control over the transport boundary. Benchmarking requires exact handling for:

- OpenAI Chat Completions
- OpenAI Responses
- Anthropic Messages
- provider-specific base URLs such as OpenRouter
- raw output normalization and reasoning removal
- request and scoring persistence for resume support

That makes a small in-house request framework the simpler and safer choice for this project.
