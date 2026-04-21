# STB

STB is a Rust CLI for structured LLM benchmarking.

It is designed to evaluate multiple model instances against the same test suite, optionally post-process outputs with Lua, score them with Lua or AI judges, resume interrupted runs, and export both machine-readable and human-readable reports.

## Status

The repository is in active development.

Implemented today:

- git repository and Rust workspace initialization
- command-line skeleton for `stb test`, `stb mkt`, and `stb mks`
- initial architecture and roadmap documents
- basic dry-run output and CLI tests
- initial `example/` fixtures that define the target file formats

Planned next:

- configuration loading and validation
- test bundle loading from loose files and `.stbt`
- OpenAI Chat Completions execution path
- resumable `output.json`
- Lua post-processing and scoring
- AI scoring
- OpenAI Responses support
- Anthropic Messages support

## Design Goals

- strict provider-plus-model identity
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

Current note: only argument parsing and `stb test --dry-run` are implemented so far.

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
