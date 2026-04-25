# STB — Simple Test Bench

STB is a Rust CLI for benchmarking large language models against a structured test suite.

Given a provider, a set of models, a shared test set, and optional scoring logic, STB:

1. sends each test to every selected model,
2. times every request,
3. optionally post-processes the raw output with Lua,
4. optionally scores each output with Lua scorers, AI judge scorers, or both,
5. persists everything to `output.json` so interrupted runs can resume,
6. writes CSV and JSON report summaries at the end of the run.

All communication with the model provider is HTTP only. STB does not upload anything back to the provider except the request you explicitly configure.

---

## Installation

Requires a working Rust toolchain (`cargo`).

Clone and build in release mode:

```bash
git clone https://github.com/EvensonCalder/simple-testbench.git stb
cd stb
cargo build --release
```

The binary is produced at `./target/release/stb`. Copy it anywhere on your `PATH`, or invoke it directly.

During development you can also run:

```bash
cargo run -- <args>
```

---

## Quick start

1. Create a working directory with the config files described in [Configuration files](#configuration-files). The `example/` directory in this repo is a complete, runnable scenario.
2. Export your provider API key, for example:

   ```bash
   export OPENROUTER_API_KEY="sk-or-..."
   ```

3. Dry-run to confirm the plan:

   ```bash
   stb test -i example --dry-run
   ```

4. Run the real benchmark:

   ```bash
   stb test -i example --output-dir ./runs/first
   ```

5. Inspect the outputs:

   - `./runs/first/output.json`        — per-request state (used for resume)
   - `./runs/first/score_mean.csv`     — average score per model instance
   - `./runs/first/score_std.csv`      — standard deviation per score
   - `./runs/first/duration_mean.csv`  — average request time per model instance
   - `./runs/first/results.json`       — full structured report, only when `--json` is passed

Re-running the same command with the same `--output-dir` will resume where the previous run stopped.

---

## Commands

### `stb test`

Run a benchmark session.

```
stb test [OPTIONS]
```

| Option | Description |
|---|---|
| `-i, --input <PATH>` | Directory with `providers.json`, `models.json`, and any optional files. Default: `.` |
| `-t, --test-archive <FILE>` | Use a packaged `.stbt` archive instead of loose `system_prompts.json` / `tests.json`. |
| `-s, --score-archive <FILE>` | Use a packaged `.stbs` archive for scoring assets. |
| `--provider <ID>` | Only run the given provider. |
| `--model <ID>` | Only run the given model id. Requires `--provider`. Matches all instances of that id. |
| `--repeat <N>` | Override `repeat` on every test. |
| `--concurrency <N>` | Cap global concurrency (provider `concurrency` is still honored). |
| `--retry <0..=3>` | Per-request retry count on failure. Default: `3`. |
| `--json` | Additionally write `results.json` with aggregates. |
| `--format <table \| json>` | How to print the run summary to stdout. Default: `table`. |
| `--output-dir <PATH>` | Directory to write `output.json` and reports into. Default: auto-named `stb_out_<ts>/`. |
| `--fresh` | Delete any existing `output.json` / reports in the output directory and start over. |
| `--npp` | Disable `post_process.lua` for this run. |
| `--dry-run` | Resolve config and plan, but do not make any requests. |
| `--verbose` | Print each request attempt and request failure. |
| `--help`, `--version` | Standard. |

Failure behavior:

- Requests are executed concurrently per provider. Each provider runs up to its configured `concurrency`; `--concurrency` caps total in-flight benchmark requests across all providers.
- Provider `rpm` is enforced as a per-provider request start rate, so concurrent workers do not exceed the configured requests-per-minute ceiling.
- Each request uses the retry schedule `3s, 10s, 30s` (truncated to `--retry`).
- If all attempts fail for one test, that model instance is **disabled for not-yet-started requests** in the rest of the run. Remaining tests for it are recorded as `skipped_model_disabled`; already in-flight requests may still finish.
- `output.json` is written through an atomic same-directory temporary file and rename, so an interrupted write should leave the previous complete file intact.
- Failed requests do not contribute to the timing average.

### `stb mkt`

Package test inputs into a reproducible `.stbt` archive.

```
stb mkt -i ./my-scenario -o my-scenario.stbt
```

Archives `system_prompts.json` and `tests.json`.

### `stb mks`

Package scoring inputs into a `.stbs` archive.

```
stb mks -i ./my-scenario -o my-scenario.stbs
```

Archives `scoring.json`, `post_process.lua`, every Lua scorer, and every AI scorer JSON referenced from `scoring.json`.

### Mixing archives and loose files

`stb test` reads loose files from `-i`, and overrides individual concerns with an archive if `-t` or `-s` is supplied. This is convenient for distributing a frozen test set while keeping the provider/models configuration local.

---

## Configuration files

All files are standard JSON unless noted. The filenames are exact.

### `providers.json` (required)

Defines providers. A provider may expose one or more API styles through an `endpoints` map.

```json
{
  "providers": [
    {
      "provider_id": "openrouter",
      "env_key": "OPENROUTER_API_KEY",
      "concurrency": 4,
      "rpm": 60,
      "endpoints": {
        "openai_chat_completions": "https://openrouter.ai/api/v1",
        "openai_responses":        "https://openrouter.ai/api/v1/responses",
        "anthropic_messages":      "https://openrouter.ai/api/v1/messages"
      }
    }
  ]
}
```

Fields:

| Field | Required | Notes |
|---|---|---|
| `provider_id` | yes | Unique. Used with `model_id` to identify models. |
| `key` | one of `key` / `env_key` | Literal API key. Avoid committing secrets. |
| `env_key` | one of `key` / `env_key` | Name of the environment variable to read the API key from. |
| `concurrency` | yes | Max concurrent in-flight requests to this provider. Must be `> 0`. |
| `rpm` | yes | Requests-per-minute ceiling. Must be `> 0`. |
| `endpoints` | yes | Base URL per supported `api_style`. Only styles actually used by models need to be present. |

`api_style` values:

- `openai_chat_completions` — endpoint base ends before `/chat/completions`. STB appends the correct path.
- `openai_responses`        — endpoint is the full `/responses` URL.
- `anthropic_messages`      — endpoint is the full `/messages` URL.

For all styles, STB keeps only the final assistant text. Reasoning / thinking blocks are discarded.

### `models.json` (required)

Each entry is one **model instance** that will be benchmarked.

```json
{
  "models": [
    {
      "provider_id": "openrouter",
      "model_id": "z-ai/glm-5.1",
      "api_style": "openai_responses",
      "streaming": true,
      "timeout": 300
    },
    {
      "provider_id": "openrouter",
      "model_id": "z-ai/glm-5.1",
      "api_style": "openai_responses",
      "temperature": 0.7,
      "streaming": false,
      "timeout": 600
    }
  ]
}
```

Rules:

- Required fields: `provider_id`, `model_id`, `api_style`.
- `temperature` is optional. If omitted, STB does **not** send the field, which is required by some providers/models that reject `temperature`.
- `max_output_tokens` is optional. If omitted, STB does **not** send a token cap, which lets reasoning-heavy models use their own default. Set it explicitly when you need a hard ceiling.
- `streaming` is optional and defaults to `true`. STB sends provider streaming requests and reads Server-Sent Events so long-running reasoning models can keep the connection active. Set it to `false` for providers that do not support streaming.
- `timeout` is optional and defaults to `300` seconds. It is a per-model HTTP read timeout: if the provider connection produces no response data for this many seconds, the attempt fails and follows the retry policy.
- Any additional provider-specific fields can be added at the top level of a model entry; except for STB-reserved fields like `streaming` and `timeout`, they are forwarded verbatim as JSON fields in the request body.
- Two entries with the same `provider_id + model_id` are treated as **different instances** if their parameters differ. This lets you benchmark the same model at, e.g., two `temperature` values in one run.

STB computes a stable `model_instance_id` from `provider_id`, `model_id`, and the effective parameter set. That instance id shows up in `output.json` and every report so results with different parameters never get averaged together.

### `system_prompts.json` (optional)

```json
{
  "system_prompts": [
    {
      "id": "todo_json_v1",
      "text": "Return JSON only with keys todo, time, location. Use null for missing fields."
    }
  ]
}
```

`id` must be unique. It is referenced from each test case.

### `tests.json` (optional)

```json
{
  "tests": [
    {
      "id": "todo-001",
      "system_prompt": "todo_json_v1",
      "input": [
        { "type": "text", "text": "Remind me to buy milk tomorrow at 7pm at Walmart." }
      ],
      "repeat": 1
    }
  ]
}
```

- `input` is an array of content blocks. Today only `{"type":"text"}` is supported; the array shape is there so multimodal content can be added without a format break.
- `repeat` defaults to `1`.
- `--repeat N` on the command line overrides `repeat` for every test.

### `scoring.json` (optional)

```json
{
  "scoring": [
    { "name": "json",              "kind": "lua", "file": "score_json.lua"        },
    { "name": "extraction_quality","kind": "ai",  "file": "score_extract_ai.json" }
  ]
}
```

Each entry declares one scorer:

- `name` must be unique in the run. It becomes the row key in `score_mean.csv` and `score_std.csv`.
- `kind` is `lua` or `ai`.
- `file` is resolved relative to the input directory (or inside the `.stbs` archive).

---

## Lua scripting

STB uses an embedded Lua 5.4 runtime (no external Lua install required).

### `post_process.lua`

Optional. When present, it runs after every successful model response and before scoring.

Contract: the script must **return a function** that takes the raw model output string and returns a table.

```lua
return function(raw_output)
  local body = tostring(raw_output or "")
  body = body:gsub("^%s+", ""):gsub("%s+$", "")
  body = body:gsub("^```json%s*", ""):gsub("^```%s*", ""):gsub("%s*```$", "")

  return {
    output = body,
    -- optional:
    -- retry = false,
    -- max_retry = 0,
  }
end
```

Table fields:

| Field | Type | Default | Effect |
|---|---|---|---|
| `output` | string | raw output unchanged | Value passed to all scorers and persisted as `processed_output`. |
| `retry` | bool | `false` | If `true`, STB will rerun the model request for this test case. |
| `max_retry` | integer | `0` | Caps how many times `retry=true` can trigger a rerun for the same test. |

Use `--npp` to disable post-processing for a run while leaving the file in place.

### Lua scorer

Contract: return a function that takes the processed output string and returns an integer from `0` to `100`.

```lua
return function(processed_output)
  if processed_output:sub(1, 1) == "{" then
    return 100
  end
  return 0
end
```

Values outside `[0, 100]` are recorded as a scorer failure for that record without aborting the run.

### AI scorer

AI scorers are declared with `"kind": "ai"` and a JSON config file:

```json
{
  "provider_id": "openrouter",
  "model_id": "z-ai/glm-5.1",
  "api_style": "openai_responses",
  "streaming": true,
  "timeout": 300,
  "system_prompt": "You are grading extraction quality. Return JSON only in the shape {\"score\": 0-100, \"reason\": \"short explanation\"}."
}
```

At judge time STB sends the provider a prompt containing the benchmark test input and the candidate model output, asks the judge to return strict JSON, and tolerates responses wrapped in ```` ```json ... ``` ```` fences.

Same rules as regular models apply for the judge:

- `temperature`, `max_output_tokens`, `streaming`, and `timeout` are optional.
- Omit them when you want the provider default.

If the judge returns invalid JSON or an out-of-range score, that score is recorded as failed on the candidate record but the rest of the run continues.

---

## Reports

After every `stb test` run, STB writes:

### `output.json`

Durable state. One entry per `(provider_id, model_instance_id, test_id, repeat_index)`.

Each entry stores:

- `status`: `success`, `failed`, or `skipped_model_disabled`.
- `attempts`: total request attempts (including retries).
- `elapsed_ms`: total request time in milliseconds. Kept only when the request succeeded.
- `output_text`: raw model output.
- `processed_output`: post-processed output (equal to raw if there was no post-process).
- `post_process_applied`, `post_process_retries`: diagnostics for Lua post-processing.
- `scores[]`: one item per scorer with `name`, `kind`, `status`, `score`, optional `details`, optional `error`.

Resuming a run reuses every record in `output.json`. If scoring assets changed between runs, existing records will keep their raw output and have only the missing scorers recomputed.

### `score_mean.csv` / `score_std.csv`

Per `(provider_id, model_id, model_instance_id, score_name)`, the mean and standard deviation of successful scores. Floats rounded to 4 decimal places.

### `duration_mean.csv`

Per `(provider_id, model_id, model_instance_id)`, the average request duration in milliseconds.

- Failed requests are **ignored** when computing the average.
- If a model instance had **zero** successful requests, the average is written as `N/A`.

### `results.json` (only with `--json`)

Full structured report: raw records, score aggregates, and duration aggregates, all in one file. Convenient for downstream analysis.

### Terminal summary

After the run STB prints either a two-section table (duration + scores) or the equivalent JSON, depending on `--format`.

---

## Resuming and re-running

- Reusing the same `--output-dir` resumes. Already-completed `(provider, model_instance, test, repeat_index)` tuples are not re-requested; only missing ones are filled in. Missing scores on previously-successful records are also computed during resume without re-sending the benchmark request.
- `--fresh` wipes `output.json` and the report files in the output directory before starting.
- Because `model_instance_id` is parameter-sensitive, changing `temperature` or `max_output_tokens` on a model produces a new instance. Old records for the old instance are preserved.

---

## Environment variables

| Name | Used for |
|---|---|
| `OPENROUTER_API_KEY` (or any name referenced by a provider's `env_key`) | Provider API key. STB never logs its value. |

STB reads HTTP proxy variables (`https_proxy`, `http_proxy`, `no_proxy`) via `reqwest`'s default behavior, so standard proxy setups work without extra configuration.

---

## Troubleshooting

**`Missing Authentication header` from the provider**
Your API key is not reaching the provider. Check that `env_key` matches an exported env var, that the value is not quoted incorrectly in your shell, and that the key itself is valid for the endpoint you configured.

**Reasoning-only response, no final text**
Some reasoning models can spend their entire output budget on hidden reasoning and return no final text. Give the model more room by setting `max_output_tokens` explicitly in `models.json`.

**Request timeout while the model is still thinking**
Streaming is enabled by default so providers can keep the connection alive while a model reasons. If a provider sends no bytes before the read timeout, increase that model's `timeout` value in `models.json`, or set `streaming` to `false` only when the provider cannot stream.

**`duration_mean.csv` shows `N/A`**
Every request for that model instance failed. Look at the `error` field of the related records in `output.json`.

**`AI scorer response was not valid JSON`**
Tighten the system prompt of the AI scorer so it produces strict JSON, or keep fenced JSON — STB already strips ```` ```json ```` fences automatically.

---

## Example

See `example/` for a full scenario that extracts `todo` / `time` / `location` as JSON from natural-language reminders. It demonstrates:

- one provider with all three API styles,
- four model instances across those styles,
- a Lua post-processor,
- a Lua scorer,
- an AI judge scorer.

Run it with:

```bash
stb test -i example --output-dir runs/example --json
```

---

## Further reading

Design and implementation notes for contributors:

- `docs/architecture.md`
- `docs/file-formats.md`
- `docs/development.md`
- `docs/roadmap.md`

---

## License

STB is distributed under the [MIT License](LICENSE).

Copyright © 2026 EvensonCalder.
