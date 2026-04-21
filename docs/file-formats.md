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
      "extra": {
        "seed": 7
      }
    }
  ]
}
```

Rules:

- `provider_id + model_id` is the unique logical identity
- `api_style` selects the request adapter
- provider-specific extras are stored in `extra`

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

Planned responsibilities:

- store each request and each score result
- assign a stable random id to every request-response record
- allow interrupted runs to resume safely
- preserve enough metadata to match scores to outputs

## Output reports

Planned output files:

- `results.json`
- `score_mean.csv`
- `score_std.csv`

CSV outputs will contain only aggregate averages and standard deviations per model and score item, rounded to four decimal places.
