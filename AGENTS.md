# Repository Guidelines

## Project Structure & Module Organization

Rigel is a Rust 2024 CLI for OpenAI-compatible model APIs. `src/main.rs` is the
only composition root: it parses CLI input, creates concrete implementations,
composes use cases through infrastructure traits, and dispatches requests to controllers. Modules:

- `src/cmd/` — CLI parsing
- `src/infrastructure/` — application ports and capability boundaries
- `src/entities/` — domain entities and value objects
- `src/controllers/` — input/output boundaries and concrete external integrations
- `src/use_cases/` — provider-neutral application workflows and state transitions
- `src/prompts/` — static model-facing prompt text
- `src/tools/` — agent-callable tools
- `src/shared/` — domain-neutral technical primitives, including terminal loaders

Infrastructure traits describe capabilities without provider or terminal framework types;
entities hold domain data; use cases implement workflows; controllers own
external integrations and boundary translation. There is no adapters layer.
Keep all cross-layer concrete wiring at `src/main.rs`.

Keep static model-facing prompt text in Markdown files under `src/prompts/` and
include it at compile time with `include_str!`. Keep Rust prompt modules focused
on composition, interpolation, and prompt behavior.

Each module exports via its nearest `mod.rs`. Dependencies in `Cargo.toml`, reproducible versions in `Cargo.lock`.

Declare every production submodule in its nearest `mod.rs` using `pub mod`.
The crate root (`main.rs`) is the only exception for top-level module declarations.
Do not declare production submodules from implementation files. Do not re-export
types with `pub use` or `pub(crate) use`; import them through their declaring
module path, such as `use crate::entities::selected_model::selection::SelectedModel`.

- Keep every directory to at most seven files, excluding `mod.rs` from the count.
  Before creating an eighth file, reconsider the decomposition: move a coherent
  group of related files into a subdirectory and export only the required items
  from that subdirectory's `mod.rs`.

Domain entities and value objects must live in `src/entities/`. Do not place
domain entities in `src/shared/`; that directory is reserved for cross-cutting
technical infrastructure and shared application mechanisms.

## Chat Run Loop Architecture

`Chat::run` (`src/use_cases/chat/workflow.rs`) is the interactive command-loop boundary
and must remain straightforward to follow. It delegates user-turn model work through
infrastructure traits and the opaque model-session boundary. Rig/OpenAI event translation
stays under `src/controllers/model_runtime/`; chat state transitions, recovery, compaction,
history mutation, and command handling stay under `src/use_cases/chat/`.

- `Chat::run` delegates model work through `ModelSession` and application history/interaction traits and does not expose provider types to callers.
- Keep provider event translation and thin Rig hooks under `src/controllers/model_runtime/`; do not move business policy there.
- Keep the application boundary linear and easy to inspect; detailed parsing, formatting, state transitions, history mutation, recovery, and error handling belong to named use-case operations behind infrastructure traits.
- Controllers may implement external capabilities, but must not construct another layer's concrete collaborators; composition belongs in `src/main.rs`.
- Refactor new logic out of `run` if it makes the loop harder to read. A reviewer must understand one user turn from `run` without reading implementation details.

## Built-in Tool Architecture

Rigel currently registers ten built-in tools from the composition root in
`src/main.rs`; the Rig runtime receives them through the registration boundary:

- `src/tools/tool_ask_user/`
- `src/tools/tool_run_command/`
- `src/tools/tool_read_file/`
- `src/tools/tool_edit_file/`
- `src/tools/tool_write_file/`
- `src/tools/tool_search_files/`
- `src/tools/tool_glob_files/`
- `src/tools/tool_fetch_url/`
- `src/tools/tool_spawn_subagent/`
- `src/tools/tool_mark_goal_complete/`

Use the dedicated workspace tools for ordinary bounded file reads, literal
searches, glob discovery, and exact edits. `run_command` remains the fallback
for builds, tests, git, programs, and capabilities not covered by a dedicated
tool. Keep tool output bounded and return actionable model-facing diagnostics on
failures. Keep the existing permissions mechanism unchanged; filesystem-changing
operations remain subject to permission checks.

## Tool Error Recovery

Recovery handles weak models where malformed and unnecessary tool calls are expected.

- After an error, accept a corrected call or non-empty final answer. Retry only responses containing neither. Prompts must forbid unrelated tool calls.
- Do not treat arbitrary success as recovery. Only a later non-no-op success from the same tool clears its pending failure (`not_found` does not resolve a previous failure). A final answer may still close the pending state.
- Recovery is scoped to one agent run. Do not reconstruct failures from old chat history.
  Subagent orchestration is provider-neutral application behavior in
  `src/use_cases/subagent/workflow.rs`; its provider session factory belongs under
  `src/controllers/model_runtime/` and creates fresh, nonpersistent task history.
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

## API Design

1. When designing a struct API, its constructor must accept no more than three arguments (excluding `self`). If more context is required, split responsibilities or use a dedicated context object.
2. If a constructor field is optional, it must not be a constructor argument. Add it through a builder-pattern method instead.

## Domain Terminology

### Session

Session is one run of one agent created by a single `Agent::build_agent` call. Rebuilding an agent starts a new session and resets state that is held only in memory for the previous agent run.

## Design and Complexity Constraints

Treat the following as mandatory contribution rules:

- One responsibility per struct. Methods must not accumulate unrelated logic.
- At most three function/method arguments (excluding `self`). Redesign the API rather than growing parameter lists.
- Justify every new struct by domain or a business rule. No structs as mere implementation conveniences.
- Shallow control flow: no more than two nested `if`/`match` levels. Extract named operations for deeper branching.
- At most 30 lines of code per function/method. Decompose into named operations rather than compressing.
- At most 300 lines of non-test Rust code per file, including imports. `#[cfg(test)]` test modules are excluded from this limit regardless of their size.
- Every newly written public struct field or method must have an immediately preceding `///` doc comment. The comment must explain the field or method's purpose, responsibility boundary, usage, and relevant edge cases. Documentation comments count toward the 300-line production-code limit.

## Testing Guidelines

Tests use Rust's built-in test framework, placed in `#[cfg(test)] mod tests`. Name after observable behavior (`reasoning_without_answer_requires_recovery`). Cover success, failure, and edge cases. No coverage threshold; prioritize meaningful behavioral assertions. Unit tests must not perform any I/O — no temp files, directories, or external services. Extract pure logic; inject and mock I/O boundaries.

## Commit & Pull Request Guidelines

Commits use short, imperative, title-cased subjects (`Refactor chat loop`). Keep focused; avoid mixing cleanup with feature changes. PRs explain the behavior change, list verification commands, and link issues. Include terminal output or screenshots for CLI changes; call out API/server assumptions needed for manual testing.
