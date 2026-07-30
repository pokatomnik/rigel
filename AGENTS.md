# Repository Guidelines

## Project Structure & Module Organization

Rigel is a Rust 2024 CLI for chatting with models served by Ollama. The entry
point is `src/main.rs`. Command-line parsing lives in `src/cmd/`, request
orchestration in `src/controllers/`, and application workflows in
`src/use_cases/` (currently chat and model selection). Shared terminal behavior
belongs in `src/shared/`; LLM prompts and prompt text are under `src/prompts/`;
agent-callable tools live in `src/tools/`. Keep each module exported through its
nearest `mod.rs`. Dependencies and crate metadata are maintained in
`Cargo.toml`, with reproducible versions recorded in `Cargo.lock`.

## Build, Test, and Development Commands

- `cargo run -- --help` displays the available CLI options.
- `cargo run -- --base-url http://localhost:11434` starts Rigel against a local
  Ollama server; use `--api-key` only when the endpoint requires it.
- `cargo build` compiles the debug binary to `target/debug/rigel`.
- `cargo test --all-targets` runs the complete test suite.
- `cargo fmt --all -- --check` verifies standard Rust formatting.
- `cargo clippy --all-targets --all-features` reports common correctness and
  maintainability issues.

## Coding Style & Naming Conventions

Use `rustfmt` defaults (four-space indentation and trailing commas in multiline
constructs). Follow Rust conventions: `snake_case` for modules, functions, and
variables; `PascalCase` for structs and traits; and `SCREAMING_SNAKE_CASE` for
constants. Prefer small modules aligned with the existing controller/use-case
boundaries. Return `anyhow::Result` at application boundaries and use concrete
error types where library-style APIs benefit from them.

## Testing Guidelines

Tests currently use Rust's built-in test framework and sit beside the code in
`#[cfg(test)] mod tests` blocks. Name tests after observable behavior, such as
`reasoning_without_answer_requires_recovery`. Add tests for success, failure,
and edge cases when changing chat state or recovery logic. No coverage threshold
is configured; prioritize meaningful behavioral assertions. Unit tests must not
perform any I/O, including filesystem or network access. Do not create temporary
files or directories or contact external services from unit tests. Extract pure
logic or inject and mock I/O boundaries instead.

## Commit & Pull Request Guidelines

Recent commits use short, imperative, title-cased subjects such as
`Refactor chat loop`. Keep commits focused and avoid mixing unrelated cleanup
with feature changes. Pull requests should explain the behavior change, list
verification commands, and link relevant issues. Include terminal output or
screenshots when CLI interaction changes, and call out any Ollama model or
server assumptions needed for manual testing.
