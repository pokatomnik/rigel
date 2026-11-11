# Rigel

A Rust 2024 CLI agent for interacting with OpenAI-compatible LLM APIs.

## Project Overview

Rigel is a conversational LLM agent that integrates shell and network tools into an interactive CLI session. It connects to OpenAI-compatible servers and runs commands from the directory where Rigel started.

## Architecture

Rigel keeps application workflows, domain data, concrete integrations, and composition separate:

- `src/main.rs` is the only composition root. It parses the CLI, creates concrete implementations, wires them through infrastructure traits, and starts the controllers.
- `src/cmd/` contains only Clap parsing and CLI DTOs.
- `src/infrastructure/` contains traits for capabilities shared across layers, without exposing provider or terminal framework types.
- `src/entities/` contains domain entities and value objects, including application requests, configuration, conversation state, goals, skills, and tool data.
- `src/controllers/` owns external-system integrations and input/output boundaries: configuration/history persistence, terminal I/O, workspace access, MCP, OpenAI/Rig sessions, and provider event translation.
- `src/use_cases/` owns provider-neutral workflows: chat turns, history state transitions, recovery policy, compaction, goals, permissions, and subagent orchestration. It depends only on entities and infrastructure traits.
- `src/tools/` contains agent-callable implementations and receives state/I/O capabilities through infrastructure traits.
- `src/prompts/` contains static model-facing prompt text; Rust prompt modules compose it with runtime values.
- `src/shared/` contains domain-neutral technical primitives.

Infrastructure contains capability traits only; entities contain domain data/state, and `shared` remains domain-neutral. Concrete integrations live under `src/controllers/` and tool-specific implementations under `src/tools/`.

## Configuration

Rigel reads the global TOML configuration from `~/.rigel/config.toml` by
default. `rigel chat --profile <PATH>` uses `<PATH>` as the global file. In
both cases Rigel also reads `<current-dir>/.rigel/config.toml` and overlays it
on the global configuration. Each explicitly configured project value overrides
the global/profile value; an omitted project value is inherited. Defaults are
used only when neither layer configures that value. Read or TOML parse errors in
the optional default-global and project files are ignored; Rigel uses the other
valid layer, or defaults if neither is usable. An explicit `--profile` is
required, so its read or parse errors are reported; project configuration remains
optional.

The MCP servers whose tools are exposed to the agent are declared in TOML:

```toml
[mcpServers.local]
type = "stdio"
command = "node"
args = ["server.js"]

[mcpServers.local.env]
RUST_LOG = "debug"

[mcpServers.remote]
type = "http"
url = "https://example.com/mcp"
```

Tool permissions use one policy per runtime tool name:

```toml
[policies."run_command"]
allow = true
```

Project policies take precedence over global policies. `allow = false` denies
the tool without showing a confirmation. When no policy exists, automatic
tools run without confirmation and other tools ask. A confirmation can deny a
call, allow it once, allow it for the project, allow it for the current agent
session, or allow it forever. Project allowances are stored in
`<current-dir>/.rigel/config.toml`; forever allowances are stored in the
`--profile` file or, without `--profile`, `~/.rigel/config.toml`. Session
allowances last until that agent is rebuilt, and deny/once decisions are not
stored. `rigel init` creates the global file without any policies.

Model-specific provider parameters can be configured under `models`. The model
key must exactly match the ID returned by `/models` (including letter case):

```toml
[models."gpt-5.5".params]
reasoning_effort = "high"

[models."qwen3".params]
think = true

[models."router-model".params.reasoning]
effort = "high"
```

Values may be strings, booleans, numbers, arrays, or nested tables. Rigel does
not determine provider or model capabilities and does not normalize these
values; it forwards them as additional top-level request fields. If the model
is not listed, or its `params` table is empty, provider defaults are used.
Transport-owned fields such as `model`, `messages`, `tools`, `tool_choice`,
`temperature`, `max_tokens`, and `stream` cannot be configured here.

For an unknown model, consult the provider's API documentation for its supported
fields and add the exact model ID from `/models` to `config.toml`. Unsupported
parameters are reported by the provider as request errors; Rigel does not retry
the request without them.

Each server entry declares its transport with the `type` key:

- `"stdio"` — launch a local process: `command` (required in the merged configuration), `args` and `env` (optional)
- `"http"` — connect to a remote endpoint: `url` (required in the merged configuration)

For a server with the same name and type in the global/profile configuration,
project settings may omit `command` or `url` and inherit them from the global
layer. Project `args` are appended to global `args`, while project environment
variables override global variables with the same name.

Read or TOML parse errors in the optional default-global and project files are
ignored; Rigel uses the other valid layer, or defaults if neither layer is
usable. An explicitly supplied `--profile` is required, so its read or parse
errors are reported. The project config remains optional even when a profile is
specified.

### Interactive commands

During `rigel chat`, the following commands are available:

- `/exit` — exit the chat
- `/skills` — choose a skill and send its instructions to the agent; `/skill` is an alias
- `/help` — show the command list
- `/editor` — write a prompt in the default editor
- `/new` — clear the conversation history
- `/compact` — compact the conversation context
- `/agent` — change agent preferences
- `/goal <text>` — pursue the exact goal autonomously until the model calls `mark_goal_complete`

After a successful provider request, Rigel automatically compacts the chat or
subagent history when the last request used at least 80% of the selected
model's context. It uses the provider's `total_tokens` and the selected model's
explicit `context_length` from `/models`; if either value is unavailable, the
automatic compaction is skipped. After compaction, the history is replaced by
one assistant summary message and the next request starts with unknown usage
again.

The prompt displays usage as `used/limit >`, using `K`, `M`, or `B` suffixes
for large values. If only the context limit is known, the prompt shows `?/1M >`;
if only usage is known, it shows `123K >`; if neither is known, it shows `> `.
`/compact` remains available for manual compaction and uses the same summary behavior.

Chat history is persisted at `~/.rigel/<sha256-of-startup-directory>/history.json`;
each startup working-directory path has its own history file. Delegated subagent
calls start with fresh, nonpersistent history containing the delegated task,
not the chat's history or context. Each delegation inherits the chat agent's
currently selected model, including after `/agent` changes it, but not its
conversation history. The built-in subagent tool set includes workspace,
command, and fetch tools; it excludes the built-in `ask_user`, `spawn_subagent`,
and `mark_goal_complete` tools. Selected MCP tools are also registered for the
provider runtime.

While a goal is active, Rigel does not read new user commands. After each completed model run it
sends an internal continuation prompt containing the original goal. These follow-up runs have the
same 12-turn limit as an ordinary run, but the number of consecutive follow-up runs is unlimited.
The original goal and internal prompts are stored in the normal conversation history. Tools that
normally require confirmation continue to request confirmation; `mark_goal_complete` is automatic.
This completion tool is exposed only to the main chat agent, not to subagents.

### Built-in skills

`configure-rigel` is always available in `/skills`. Its static source lives under
`src/prompts/skills/configure-rigel/SKILL.md`; it documents the TOML configuration
file, model parameters, MCP servers, and project and global skill directories. It
is compiled into Rigel and does not require a file in the current project or home
directory. When selected, it provides this context to
the agent and asks how it can help configure Rigel instead of asking the agent
to summarize the skill.

## Quick Start

### Installation

```bash
# Clone the repository
git clone <repository-url>
cd rigel

# Build the project
cargo build

# Run help command
cargo run -- --help
```

### Configuration Options

```bash
# Create ~/.rigel/config.toml with the default API URL
cargo run -- init

# Configure the API URL and the environment variable containing the API key
cargo run -- init --base-url <api-base-url> --api-key-env OPENAI_API_KEY

# Start an interactive chat
cargo run -- chat

# Start with an explicit TOML profile
cargo run -- chat --profile ./config.toml

# Run tests
cargo test --all-targets

# Format Rust code
cargo fmt --all -- --check

# Run clippy checks
cargo clippy --all-targets --all-features
```

## Project Structure

```
rigel/
├── src/
│   ├── main.rs        # Composition root and CLI dispatch
│   ├── cmd/           # CLI argument parsing and options
│   ├── infrastructure/ # Application ports and capability boundaries
│   ├── controllers/   # Boundary controllers and concrete external integrations
│   ├── entities/      # Domain entities and value objects
│   ├── prompts/       # Static model-facing prompt text
│   ├── shared/        # Domain-neutral technical primitives
│   ├── tools/         # Agent-callable tool implementations
│   └── use_cases/     # Application workflows
├── Cargo.toml         # Rust package metadata and dependencies
├── Cargo.lock         # Resolved dependency versions
└── README.md          # This file
```

## Key Components

### Command-Line Interface

The CLI entry point is defined in `src/cmd/cli.rs`. It provides the `init`
and `chat` subcommands:

- `rigel init [--base-url <URL>] [--api-key-env <NAME>]` — create the default configuration
- `rigel chat [--profile <PATH>]` — start a chat, optionally with an explicit configuration

```bash
cargo run -- chat --profile ./config.toml
```

### CLI Controllers and Concrete Integrations

`src/controllers/chat_controller/controller.rs` contains a thin `ChatController`. It receives a `ChatRequest`, writes the welcome message through `ApplicationOutput`, and invokes the provider-neutral chat workflow through `ChatRunner`. `InitController` follows the same contract-only boundary for configuration initialization.

Concrete configuration/history, terminal, workspace, MCP, and Rig/OpenAI integrations live under `src/controllers/`; application workflows live under `src/use_cases/`. `src/main.rs` selects and wires them. Controllers and use cases communicate through infrastructure traits and entities, never through concrete implementations owned by another layer.

### Built-in Tools

Rigel currently provides ten built-in tools. Their implementations live under `src/tools/`; `src/main.rs` creates both chat and subagent tool sets and the Rig runtime receives them through a type-erased registration boundary:

| Tool                 | Implementation path                  | Description                                                                             |
| -------------------- | ------------------------------------ | --------------------------------------------------------------------------------------- |
| `ask_user`           | `src/tools/tool_ask_user/`           | Ask one question with selectable answers or required free-form input                    |
| `run_command`        | `src/tools/tool_run_command/`        | Run bounded shell commands for builds, tests, git, programs, and uncovered capabilities |
| `read_file`          | `src/tools/tool_read_file/`          | Read bounded UTF-8 text with numbered lines                                             |
| `edit_file`          | `src/tools/tool_edit_file/`          | Replace one exact unique fragment with a bounded diff                                   |
| `write_file`         | `src/tools/tool_write_file/`         | Create or fully replace a UTF-8 text file                                               |
| `search_files`       | `src/tools/tool_search_files/`       | Search bounded file contents for literal text                                           |
| `glob_files`         | `src/tools/tool_glob_files/`         | Find bounded workspace files by a relative glob pattern                                 |
| `fetch_url`          | `src/tools/tool_fetch_url/`          | Fetch bounded text from a known HTTP or HTTPS URL                                       |
| `spawn_subagent`     | `src/tools/tool_spawn_subagent/`     | Run an autonomous subagent for a delegated task                                         |
| `mark_goal_complete` | `src/tools/tool_mark_goal_complete/` | End the active `/goal` mode with a truthful work report                                 |

`read_file` reads only bounded, ordinary UTF-8 text from relative paths inside the startup workspace and runs automatically; `search_files` performs bounded, literal, case-sensitive content searches in files or directories inside that workspace and also runs automatically; `glob_files` performs bounded filename discovery in that workspace. Prefer these tools instead of shell commands or pipelines for ordinary file inspection and search. After reading or searching, use `edit_file` for one exact unique replacement in an existing UTF-8 file; it requires confirmation, verifies the file was not changed between reads, writes the result directly, and returns only a bounded diff. Use `write_file` for a consciously complete new file or full replacement; it requires confirmation, creates missing parent directories inside the workspace, never appends, and rejects content above its server limit. `ask_user` is automatic and is available only to the main chat agent: it presents the model's ordered answers plus `Something else`; choosing the latter keeps the same tool call open until the user enters non-empty free text. It does not add a separate user turn and is available during `/goal` when the model needs the user's choice. Its automatic permission avoids a confirmation dialog; an explicit `allow = false` policy can still deny it without prompting. `run_command` remains the fallback for builds, tests, git, programs, and capabilities not covered by these dedicated tools; it keeps the existing shell, startup directory, 30-second timeout, output cap, and permission confirmation policy. Other filesystem changes remain subject to the existing tool permission mechanism.

### Use Cases

- **Chat**: `src/use_cases/chat/workflow.rs` runs the interactive command loop through infrastructure boundaries; named operations under `src/use_cases/chat/` handle turns, history, recovery, and compaction.
- **Initialization**: `src/use_cases/init/` creates the configured profile.
- **Model runtime**: `src/controllers/model_runtime/model_selector/` adapts provider model discovery and selection; controller runtime code translates Rig events and supplies provider capabilities.
- **Recovery and compaction**: `src/use_cases/chat/recovery/` owns turn-recovery policy, while `src/use_cases/chat/compaction/` owns history-compaction policy. Provider-backed summarization stays under `src/controllers/model_runtime/compaction/`.
- **Permissions and goals**: provider hooks live under `src/controllers/model_runtime/permissions/`; policy decisions and goal state transitions are handled by `src/use_cases/permissions/` and `src/use_cases/goal/`.
- **Subagents**: `src/use_cases/subagent/workflow.rs` implements `RunSubagent`; `src/controllers/model_runtime/subagent_session_factory.rs` creates a fresh provider session and nonpersistent history for each delegation.

## Development Guidelines

### Coding Style

- Use Rust 2024 edition
- Four-space indentation with trailing commas in multiline constructs
- `snake_case` for modules, functions, and variables
- `PascalCase` for structs and traits
- `SCREAMING_SNAKE_CASE` for constants

### Error Handling

Return `anyhow::Result` at application boundaries. Use concrete error types where library-style APIs benefit from them. Errors should be intelligible and provide actionable next steps.

### Testing

- Use Rust's built-in test framework
- Name tests after observable behavior
- Write tests for success, failure, and edge cases
- Unit tests must not perform I/O operations

## Contributing

Contributions are welcome! Please follow these guidelines:

1. Keep commits focused and concise
2. Use title-cased imperative subjects (e.g., "Refactor chat loop")
3. Document behavior changes in pull requests
4. Include verification commands for testing
5. Link to relevant issues

## License

This project is licensed under the MIT License. See the LICENSE file for details.
