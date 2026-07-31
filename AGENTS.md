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

## Chat Run Loop Architecture

`Chat::run` in `src/use_cases/chat/chat.rs` is core project functionality and
must remain a simple, linear orchestration loop. Treat this as a strict
contribution rule, not a style preference.

- Keep `Chat::run` focused on sequencing the high-level chat steps: read user
  input, run the model stream, choose the completion or recovery path, report
  the turn outcome, and continue or exit.
- Any new substantive step added to the chat loop must be implemented as a
  clearly named method on `Chat` and invoked from `run`. This includes parsing,
  formatting, state transitions, history mutation, recovery preparation, and
  error-handling details.
- Do not add nested workflow logic, multi-step transformations, or detailed
  branch bodies directly to `run` unless they are strictly necessary to express
  the top-level control flow.
- If a change makes `run` harder to read from top to bottom, refactor the new
  logic out before considering the change complete. A reviewer should be able
  to understand the full lifecycle of one user turn from `run` without reading
  implementation details there.

## Filesystem Tool Architecture

- A tool may access only the workspace in which Rigel started. Capture and
  canonicalize that root when constructing the tool. Accept workspace-relative
  paths only; reject absolute paths and `..`. Resolve existing ancestors and
  symlinks before I/O so neither source nor destination can escape the root.
  Explicitly protect the workspace root from delete or move operations.
- Use `tokio::fs` for every filesystem operation. Keep path validation,
  matching, formatting, hashing, and edit planning as pure functions where
  practical.
- Give each tool one narrow responsibility. `list_directory` inspects one
  directory, `find_paths` recursively matches path globs, `search_text` searches
  file contents, `stat` returns metadata, and `read_file` returns content.
  Search tools must not grow into general filesystem APIs.
- Tool descriptions and successful results must be short, deterministic, and
  explicit about what changed. Return normalized workspace-relative paths,
  never canonical absolute paths. Sort collections deterministically.
- Errors are model-facing recovery data. State what failed, why, which path or
  argument caused it, and the next corrective action. Prefer stable structured
  codes such as `PATH_NOT_FOUND`, `PATH_OUTSIDE_WORKSPACE`, or `INVALID_GLOB`
  where the tool has structured output. Never hide an I/O error behind a generic
  failure message.
- Preserve idempotent outcomes as explicit non-errors when the contract calls
  for them, for example deleting a missing directory returns `not_found`.
  Conversely, creating an existing file is an error and must direct the model
  to `apply_patch`.
- File reads are UTF-8 and return a SHA-256 `revision`. `apply_patch` requires
  that revision, validates every edit before writing, evaluates edits against
  the same original text, rejects missing/ambiguous/overlapping matches, and
  commits atomically. On success it returns the new revision and asks the model
  to read the file again.
- Recursive search skips binary files when searching a directory but rejects a
  directly selected binary file. Built-in search exclusions live in data files
  included with `include_str!`, not duplicated as Rust literals. Keep search
  result limits bounded and report truncation.

## Tool Error Recovery

Recovery is designed for weak models, where both malformed and unnecessary tool
calls are expected. A tool error requires an intelligible next step, not
necessarily another tool call.

- After an error, accept either a corrected tool call or a non-empty final
  answer when the failed call was unnecessary or the user request is already
  complete. Retry only responses containing neither. Prompts must explicitly
  forbid unrelated calls made only to clear an error state.
- Do not treat arbitrary success as recovery. Only a later, non-no-op success
  from the same tool clears its pending failure. In particular, `not_found`
  does not resolve a previous failure. A final answer may still close the
  pending state.
- Recovery state is scoped to one Rig agent run. Do not reconstruct pending
  failures by scanning old chat history: an older error may already have been
  intentionally closed by a final answer.
- Budgets are hard and do not reset after a successful call: at most 12 model
  turns, 6 failed tool executions, 2 identical failures, 2 empty recovery
  responses, and 2 invalid-tool retries. Detect a repeated A-B-A-B invocation
  cycle, including error/no-op cycles.
- When a failure budget or cycle guard fires, set `ToolChoice::None` for the
  following completion and request a concise final report of what succeeded and
  what could not be completed. If the model still emits tools, stop after the
  invalid-call budget instead of reopening an unbounded loop.
- `ToolRecoveryHook` writes small `rigel_tool_status` markers into model-visible
  tool results. `StreamOutputState` consumes those markers but must never
  suppress a non-empty final answer merely because an earlier tool failed.

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
