# Repository Guidelines

## Project Structure & Module Organization

Rigel is a Rust 2024 CLI for OpenAI-compatible model APIs. Entry point: `src/main.rs`. Modules:

- `src/cmd/` — CLI parsing
- `src/controllers/` — request orchestration
- `src/use_cases/` — application workflows
- `src/shared/` — shared terminal behavior
- `src/entities/` — domain entities and value objects
- `src/prompts/` — LLM prompts and text
- `src/tools/` — agent-callable tools

Keep static model-facing prompt text in Markdown files under `src/prompts/` and
include it at compile time with `include_str!`. Keep Rust prompt modules focused
on composition, interpolation, and prompt behavior.

Each module exports via its nearest `mod.rs`. Dependencies in `Cargo.toml`, reproducible versions in `Cargo.lock`.

Declare every production submodule in its nearest `mod.rs` using `pub mod`.
The crate root (`main.rs`) is the only exception for top-level module declarations.
Do not declare production submodules from implementation files. Do not re-export
types with `pub use` or `pub(crate) use`; import them through their declaring
module path, such as `use crate::entities::selected_model::SelectedModel`.

- Keep every directory to at most seven files, excluding `mod.rs` from the count.
  Before creating an eighth file, reconsider the decomposition: move a coherent
  group of related files into a subdirectory and export only the required items
  from that subdirectory's `mod.rs`.

Domain entities and value objects must live in `src/entities/`. Do not place
domain entities in `src/shared/`; that directory is reserved for cross-cutting
technical infrastructure and shared application mechanisms.

## Chat Run Loop Architecture

`Chat::run` (`src/use_cases/chat/chat.rs`) is core functionality and must remain a
simple, linear orchestration loop. This is a strict rule, not a style preference.

- `Chat::run` sequences high-level steps: read input, run stream, choose completion or recovery path, report turn outcome, continue or exit.
- Add new substantive steps as named methods on `Chat`, invoked from `run`. This includes parsing, formatting, state transitions, history mutation, recovery prep, and error handling.
- Do not add nested workflow logic or detailed branch bodies to `run` unless strictly necessary for top-level control flow.
- Refactor new logic out of `run` if it makes the loop harder to read. A reviewer must understand one user turn from `run` without reading implementation details.

## Filesystem Tool Architecture

- Filesystem interaction is performed through the `run_command` shell tool from Rigel's startup directory.
- Use shell commands such as `ls`, `rg`, `find`, `rm`, `touch`, and `patch` for inspection and changes.
- Keep the existing permissions mechanism unchanged; filesystem-changing `run_command` calls remain subject to permission checks.
- Keep command output bounded and return actionable model-facing diagnostics on failures.

## Tool Error Recovery

Recovery handles weak models where malformed and unnecessary tool calls are expected.

- After an error, accept a corrected call or non-empty final answer. Retry only responses containing neither. Prompts must forbid unrelated tool calls.
- Do not treat arbitrary success as recovery. Only a later non-no-op success from the same tool clears its pending failure (`not_found` does not resolve a previous failure). A final answer may still close the pending state.
- Recovery is scoped to one agent run. Do not reconstruct failures from old chat history.
- Hard budgets that do not reset: 12 model turns, 6 failed tool executions, 2 identical failures, 2 empty recovery responses, 2 invalid-tool retries. Detect A-B-A-B invocation cycles (including error/no-op).
- When budget or cycle guard fires: set `ToolChoice::None` for next completion and request a concise final report. If the model still emits tools, stop after invalid-call budget.
- `ToolRecoveryHook` writes small `rigel_tool_status` markers into model-visible results. `StreamOutputState` consumes them but never suppresses a non-empty final answer.

## Build, Test, and Development Commands

- `cargo run -- --help` — CLI options
- `cargo run -- init --base-url http://localhost:8000/v1` — configure Rigel for a local server
- `cargo run -- chat` — start Rigel; use `--profile <PATH>` for an explicit TOML file
- `cargo build` — debug binary at `target/debug/rigel`
- `cargo test --all-targets` — complete test suite
- `cargo fmt --all -- --check` — standard Rust formatting
- `cargo clippy --all-targets --all-features` — correctness and maintainability issues

## Documentation Contract

- Document every CLI or interactive command and every built-in skill in `README.md`.
- When the Rigel configuration contract changes, update the corresponding built-in skills in the same change.

## Coding Style & Naming Conventions

Use `rustfmt` defaults (four spaces, trailing commas). Follow Rust conventions: `snake_case` for modules/functions/variables; `PascalCase` for structs/traits; `SCREAMING_SNAKE_CASE` for constants. Prefer small modules aligned with controller/use-case boundaries. Return `anyhow::Result` at application boundaries; use concrete error types for library-style APIs.

**CRITICAL RULE**: `.unwrap()` and panicking methods (`.expect()`, etc.) are **strictly PROHIBITED**. All methods must return `Result<T, E>`.

## Encapsulation

- Logic that belongs to a structure or reads and changes its state must be implemented as a method, associated function, or trait method of that type. Do not use free-standing procedural functions for behavior owned by a structure.
- State that belongs to a structure must be stored with that structure rather than in a parallel procedural helper or unrelated registry.
- When the structure belongs to an external crate and cannot be changed, use a local wrapper or extension trait to keep the behavior at the structure boundary.

## Design and Complexity Constraints

Treat the following as mandatory contribution rules:

- One responsibility per struct. Methods must not accumulate unrelated logic.
- At most three function/method arguments (excluding `self`). Redesign the API rather than growing parameter lists.
- Justify every new struct by domain or a business rule. No structs as mere implementation conveniences.
- Shallow control flow: no more than two nested `if`/`match` levels. Extract named operations for deeper branching.
- At most 30 lines of code per function/method. Decompose into named operations rather than compressing.
- At most 500 lines of non-test code per file. Test modules excluded (may exceed 1,000 lines even if total exceeds 500).
- At most 300 characters combined for logic/structs/enums/constants in a file. Test code excluded.

## Testing Guidelines

Tests use Rust's built-in test framework, placed in `#[cfg(test)] mod tests`. Name after observable behavior (`reasoning_without_answer_requires_recovery`). Cover success, failure, and edge cases. No coverage threshold; prioritize meaningful behavioral assertions. Unit tests must not perform any I/O — no temp files, directories, or external services. Extract pure logic; inject and mock I/O boundaries.

## Commit & Pull Request Guidelines

Commits use short, imperative, title-cased subjects (`Refactor chat loop`). Keep focused; avoid mixing cleanup with feature changes. PRs explain the behavior change, list verification commands, and link issues. Include terminal output or screenshots for CLI changes; call out API/server assumptions needed for manual testing.
