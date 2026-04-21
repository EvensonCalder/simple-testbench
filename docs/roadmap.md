# STB Roadmap

## Goal

Build `stb`, a Rust CLI tool for benchmark-driven model evaluation with:

- model and provider loading from JSON
- support for OpenAI Chat Completions
- support for OpenAI Responses
- support for Anthropic Messages
- loose-file and archive-based test loading
- Lua post-processing
- Lua and AI scoring
- resumable execution state
- CSV, JSON, and terminal-table reporting

## Phases

### Phase 0: Foundation

Deliverables:

- cargo project scaffold
- git repository initialization
- module layout for config, llm, runner, scoring, archive, output, and util
- CLI skeleton for `test`, `mkt`, and `mks`
- initial docs and tests

Acceptance:

- `cargo test` passes
- `stb --help` and `stb --version` work
- `stb test --dry-run` prints a basic execution summary

### Phase 1: Config and planning

Deliverables:

- parse `providers.json`
- parse `models.json`
- validate provider-model references
- enforce `provider_id + model_id` identity
- apply CLI filters and overrides
- print a real execution plan for `--dry-run`

Acceptance:

- invalid config produces actionable errors
- `--provider`, `--model`, `--repeat`, and `--concurrency` work
- dry-run output reflects the selected execution matrix

### Phase 2: Test bundle loading

Deliverables:

- load loose `system_prompts.json`
- load loose `tests.json`
- support `.stbt` archives
- implement `stb mkt -o file.stbt`

Acceptance:

- loose files and archives load into the same in-memory model
- archive round-trip tests pass

### Phase 3: First runnable vertical slice

Scope:

- OpenAI Chat Completions only

Deliverables:

- direct in-house HTTP request path for chat completions
- request builder for chat completions
- parameter merging from model config
- retry schedule: 3s, 10s, 30s
- model-instance disable after retry exhaustion
- provider concurrency limit
- provider RPM limit
- normalized output extraction with reasoning removed
- `output.json` persistence
- resume support and `--fresh`

Acceptance:

- integration tests with mocked HTTP responses pass
- interrupted runs can resume from `output.json`

### Phase 4: Post-processing and scoring

Deliverables:

- `post_process.lua`
- Lua custom scoring
- AI scoring file loading and orchestration
- score persistence in `output.json`

Acceptance:

- post-process retry contract is enforced
- partial scoring runs can resume

### Phase 5: Reporting

Deliverables:

- `results.json`
- `score_mean.csv`
- `score_std.csv`
- terminal table output

Acceptance:

- aggregates are correct to four decimal places
- output files match documented format

### Phase 6: OpenAI Responses

Deliverables:

- OpenAI Responses request path
- response normalization for mixed content output
- reasoning discard for responses-style payloads

### Phase 7: Anthropic Messages

Deliverables:

- Anthropic Messages request path
- top-level `system` mapping
- image-compatible content handling
- thinking discard for messages-style payloads

### Phase 8: Documentation and hardening

Deliverables:

- full end-to-end example validation
- detailed user documentation
- contributor documentation
- final polish for help text and errors

## Engineering Rules

- keep the binary thin and the library testable
- keep provider transport isolated from runner logic
- avoid model-id-only identity
- prefer small vertical slices over wide incomplete stubs
- test each phase before moving to the next
