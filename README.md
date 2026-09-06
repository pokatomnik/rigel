# Rigel

A Rust 2024 CLI agent for interacting with OpenAI-compatible LLM APIs.

## Project Overview

Rigel is a conversational LLM agent that integrates shell and network tools into an interactive CLI session. It connects to OpenAI-compatible servers and runs commands from the directory where Rigel started.

## Configuration

Rigel reads the global TOML configuration from `~/.rigel/config.toml` by
default. `rigel chat --profile <PATH>` uses `<PATH>` as the global file. In
both cases Rigel also reads `<current-dir>/.rigel/config.toml` and overlays it
on the global configuration.

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

- `"stdio"` — launch a local process: `command` (required), `args` and `env` (optional)
- `"http"` — connect to a remote endpoint: `url` (required)

If the default file is missing or cannot be read or parsed, Rigel uses its
defaults. An explicitly supplied `--profile` path must exist and contain valid
TOML; errors are returned to the caller.

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

While a goal is active, Rigel does not read new user commands. After each completed model run it
sends an internal continuation prompt containing the original goal. These follow-up runs have the
same 12-turn limit as an ordinary run, but the number of consecutive follow-up runs is unlimited.
The original goal and internal prompts are stored in the normal conversation history. Tools that
normally require confirmation continue to request confirmation; `mark_goal_complete` is automatic.
This completion tool is exposed only to the main chat agent, not to subagents.

### Built-in skills

`configure-rigel` is always available in `/skills`. It documents the TOML
configuration file, model parameters, MCP servers, and project and global skill
directories. It is compiled into Rigel and does not require a file in the
current project or home directory. When selected, it provides this context to
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

## Quick Start

```bash
# Clone the repository
git clone <repository-url>
cd rigel

# Run help command
cargo run -- --help

# Start Rigel with the default configuration
cargo run -- chat

# Build the project
cargo build

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
│   ├── cmd/           # CLI argument parsing and options
│   ├── controllers/   # Request orchestration layer
│   ├── entities/      # Domain entities
│   ├── prompts/       # System prompts for LLM interaction
│   ├── shared/        # Terminal IO and shared utilities
│   ├── tools/         # Agent-callable tools
│   └── use_cases/     # Application workflows (chat, model selection)
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

### Agent Controller

Located in `src/controllers/chat_controller.rs`, the IndexController orchestrates agent operations:

- Establishes connection to an OpenAI-compatible server
- Selects available models deterministically
- Initializes chat with system prompt
- Manages conversation turns (default: 12)
- Routes built-in tools to appropriate agents

### Built-in Tools

Rigel provides nine built-in tools:

| Tool                 | Description                                                                     |
| -------------------- | ------------------------------------------------------------------------------- |
| `run_command`        | Run shell commands for builds, tests, git, programs, and uncovered capabilities |
| `read_file`          | Read bounded UTF-8 text with numbered lines                                     |
| `search_files`       | Search bounded file contents for literal text                                   |
| `edit_file`          | Replace one exact unique fragment with a bounded diff                           |
| `write_file`         | Create or fully replace a UTF-8 text file                                       |
| `fetch_url`          | Fetch bounded text from a known HTTP or HTTPS URL                               |
| `spawn_subagent`     | Run an autonomous subagent for a delegated task                                 |
| `ask_user`           | Ask one question with selectable answers or required free-form input            |
| `mark_goal_complete` | End the active `/goal` mode with a truthful work report                         |

`read_file` reads only bounded, ordinary UTF-8 text from relative paths inside the startup workspace and runs automatically; `search_files` performs bounded, literal, case-sensitive content searches in files or directories inside that workspace and also runs automatically; use them instead of shell commands or pipelines for ordinary file reading and content searches. Prefer `read_file` for inspecting a matching file. After reading or searching, use `edit_file` for one exact unique replacement in an existing UTF-8 file; it requires confirmation, verifies the file was not changed between reads, writes the result directly, and returns only a bounded diff. Use `write_file` for a consciously complete new file or full replacement; it requires confirmation, creates missing parent directories inside the workspace, never appends, and rejects content above its server limit. `ask_user` is automatic and is available only to the main chat agent: it presents the model's ordered answers plus `Something else`; choosing the latter keeps the same tool call open until the user enters non-empty free text. It does not add a separate user turn, and it is unavailable during `/goal`. Its automatic permission avoids a confirmation dialog; an explicit `allow = false` policy can still deny it without prompting. `run_command` remains the fallback for builds, tests, git, programs, and capabilities not covered by these dedicated tools; it keeps the existing shell, startup directory, 30-second timeout, output cap, and permission confirmation policy. Other filesystem changes remain subject to the existing tool permission mechanism.

### Use Cases

- **Chat**: Interactive conversation with LLM models
- **Model Selection**: Deterministic model selection based on terminal output

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
