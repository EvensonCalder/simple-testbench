# Development

## Principles

- prefer small, testable vertical slices
- keep the binary thin and logic in the library
- do not couple provider transport directly to runner state
- keep file formats explicit and documented
- avoid hidden behavior in Lua integration contracts

## Current workflow

1. Add or update a thin CLI-facing surface.
2. Implement the smallest useful library behavior behind it.
3. Add unit and integration tests.
4. Run `cargo fmt` and `cargo test`.
5. Move to the next phase only after the current slice is stable.

## Near-term implementation order

1. Config loading and validation.
2. Real dry-run planning.
3. OpenAI Chat Completions execution path.
4. Resumable `output.json`.
5. Lua post-processing and scoring.
6. Reporting.
7. OpenAI Responses.
8. Anthropic Messages.

## Dependency direction

Planned core crates:

- `clap`
- `serde`
- `serde_json`
- `thiserror`
- `anyhow`
- `tokio`
- `reqwest`
- `mlua`
- `zip`
- `csv`
- `uuid`

## Reference

`rig` and `graniet/llm` are references only.

STB should own the request framework directly instead of delegating core runtime behavior to a general-purpose LLM abstraction crate.

That keeps request coverage, normalization, retry policy, state persistence, and auditability under direct project control.
