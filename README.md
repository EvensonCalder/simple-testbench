# STB

STB is a Rust CLI for structured LLM benchmarking.

It is designed to evaluate multiple model instances against the same test suite, optionally post-process outputs with Lua, score them with Lua or AI judges, resume interrupted runs, and export both machine-readable and human-readable reports.

## Status

The repository is in active development.

Implemented today:

- git repository and Rust workspace initialization
- command-line support for `stb test`, `stb mkt`, and `stb mks`
- initial architecture and roadmap documents
- loose-file config loading and validation for providers, models, system prompts, and tests
- real dry-run planning with provider and model filters
- `.stbt` packaging and archive-backed test loading
- `.stbs` packaging for scoring assets
- live execution for `openai_chat_completions`, `openai_responses`, and `anthropic_messages`
- `output.json` persistence and resume for executed requests and scores
- per-request elapsed time persistence in `output.json`
- Lua post-processing with retry contract support
- Lua custom scoring and AI judge scoring
- `results.json`, `score_mean.csv`, and `score_std.csv` report generation
- `duration_mean.csv` report generation for average request time per model instance
- simple terminal table and JSON aggregate display
- initial `example/` fixtures that define the target file formats

## Design Goals

- strict provider-plus-model identity, with distinct instances for parameter-different configs
- support multiple request styles under one provider definition
- own the request layer so behavior stays explicit and auditable
- keep the execution core testable and modular
- make interrupted runs resumable
- keep report outputs deterministic and easy to inspect

## Command Surface

```text
stb test ...
stb mkt -o xxx.stbt
stb mks -o xxx.stbs
stb --help
stb --version
```

Current note: STB now executes all three target request styles, strips reasoning or thinking from normalized output, records per-request elapsed time, runs Lua and AI scorers during benchmark execution, persists intermediate state in `output.json`, and writes aggregate CSV and JSON reports at the end of a run.

## Build

```bash
cargo build
```

## Test

```bash
cargo test
```

## Docs

- `docs/roadmap.md`
- `docs/architecture.md`
- `docs/file-formats.md`
- `docs/development.md`

## Example

The `example/` directory contains a complete target scenario for extracting `todo`, `time`, and `location` as JSON.

## Reference

`0xPlaygrounds/rig` and `graniet/llm` are useful references for Rust LLM library design, but STB will keep its own request framework.

That is the safer fit for this project because STB needs exact control over:

- supported request styles
- provider-specific base URLs
- retry and disable semantics
- reasoning and thinking removal
- raw state persistence for resume support
- scoring orchestration
