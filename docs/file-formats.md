# File Formats

This document defines the target file contracts for STB.

## `providers.json`

Required file.

```json
{
  "providers": [
    {
      "provider_id": "openrouter",
      "key": null,
      "env_key": "OPENROUTER_API_KEY",
      "concurrency": 4,
      "rpm": 60,
      "endpoints": {
        "openai_chat_completions": "https://openrouter.ai/api/v1",
        "openai_responses": "https://openrouter.ai/api/v1/responses",
        "anthropic_messages": "https://openrouter.ai/api/v1/messages"
      }
    }
  ]
}
```

Rules:

- use `key` if present
- otherwise resolve the API key from `env_key`
- `concurrency` and `rpm` are independent limits and the stricter effective limit wins

## `models.json`

Required file.

```json
{
  "models": [
    {
      "provider_id": "openrouter",
      "model_id": "z-ai/glm-5.1",
      "api_style": "openai_responses",
      "temperature": 0,
      "max_output_tokens": 512,
      "streaming": true,
      "timeout": 300,
      "seed": 7
    }
  ]
}
```

Rules:

- `provider_id + model_id` is the unique logical identity
- `api_style` selects the request adapter
- `streaming` is optional and defaults to `true`
- `timeout` is optional, measured in seconds, and defaults to `300`
- provider-specific extras are forwarded from top-level fields that are not reserved by STB

## `system_prompts.json`

Optional.

```json
{
  "system_prompts": [
    {
      "id": "todo_json_v1",
      "text": "Return JSON only."
    }
  ]
}
```

## `tests.json`

Optional.

```json
{
  "tests": [
    {
      "id": "todo-001",
      "system_prompt": "todo_json_v1",
      "input": [
        {
          "type": "text",
          "text": "Please remind me to buy milk this Friday at 10:13 PM UTC at the supermarket."
        }
      ],
      "repeat": 1
    }
  ]
}
```

Notes:

- `input` is an array to support future multimodal content
- `repeat` defaults to `1` if omitted

## `scoring.json`

Optional.

```json
{
  "scoring": [
    {
      "name": "json",
      "kind": "lua",
      "file": "score_json.lua"
    },
    {
      "name": "extraction_quality",
      "kind": "ai",
      "file": "score_extract_ai.json"
    }
  ]
}
```

## AI scoring config

```json
{
  "provider_id": "openrouter",
  "model_id": "z-ai/glm-5.1",
  "api_style": "openai_responses",
  "temperature": 0,
  "streaming": true,
  "timeout": 300,
  "system_prompt": "You are a strict evaluator. Score from 0 to 100 as an integer and return JSON only."
}
```

At runtime, STB will append:

- the test input labeled as benchmark input
- the candidate output labeled as the scoring target
- explicit output instructions requiring an integer score from 0 to 100 in JSON format

## `post_process.lua`

Optional.

Target contract:

```lua
return function(raw_output)
  return {
    output = raw_output,
    retry = false,
    max_retry = 0
  }
end
```

Rules:

- `output` is the post-processed value used for scoring
- `retry = true` asks STB to rerun the model request
- `max_retry` caps post-process-driven retries for that test item

## Lua scoring contract

Target contract:

```lua
return function(processed_output)
  return 100
end
```

Rules:

- return an integer from `0` to `100`
- STB will validate the range and type

## `output.json`

Required runtime state file.

Current responsibilities:

- store each benchmark request record
- persist post-processed output and score results beside the raw output
- assign a stable random id to every request-response record
- allow interrupted runs to resume safely

Current record shape:

```json
{
  "id": "uuid",
  "provider_id": "openrouter",
  "model_id": "z-ai/glm-5.1",
  "model_instance_id": "stable-instance-id",
  "model_config_key": "{\"api_style\":\"openai_responses\",\"temperature\":null,\"max_output_tokens\":null,\"extra\":{}}",
  "test_id": "todo-001",
  "repeat_index": 1,
  "api_style": "openai_responses",
  "status": "success",
  "attempts": 1,
  "elapsed_ms": 482,
  "output_text": "raw model output",
  "processed_output": "post-processed output",
  "post_process_applied": true,
  "post_process_retries": 0,
  "scores": [
    {
      "name": "json",
      "kind": "lua",
      "status": "success",
      "score": 100,
      "details": null,
      "error": null
    }
  ],
  "error": null
}
```

## Output reports

Generated output files:

- `results.json` when `--json` is set
- `score_mean.csv`
- `score_std.csv`
- `duration_mean.csv`

`duration_mean.csv` contains average request duration per provider-model instance. Failed requests are ignored for timing, and model instances with no successful requests are reported as `N/A`.

Score CSV outputs contain aggregate averages and standard deviations per provider-model instance and score item, rounded to four decimal places.
