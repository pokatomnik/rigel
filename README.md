# Rigel

A Rust 2024 CLI agent for interacting with OpenAI-compatible LLM APIs.

## Project Overview

Rigel is a conversational LLM agent that integrates filesystem tools into an interactive CLI session. It connects to OpenAI-compatible servers and provides atomic, deterministic filesystem operations within the workspace context. All tools work only within the workspace where Rigel started - absolute paths and parent references are rejected for security reasons.

## Configuration

Rigel reads a TOML configuration file from `~/.rigel/config.toml` that declares the MCP servers whose tools are exposed to the agent:

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

Each server entry declares its transport with the `type` key:

- `"stdio"` — launch a local process: `command` (required), `args` and `env` (optional)
- `"http"` — connect to a remote endpoint: `url` (required)

If the file is missing or cannot be parsed, Rigel starts with no MCP servers.

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
# Connect to an OpenAI-compatible API (base URL is required)
cargo run -- -b <api-base-url>

# Optionally provide API key for authenticated endpoints
cargo run -- -b <api-base-url> -k <api-key>

# Start Rigel with a local OpenAI-compatible server
cargo run -- -b http://localhost:8000/v1

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

# Start Rigel with a local OpenAI-compatible server
cargo run -- --base-url http://localhost:8000/v1

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
│   ├── tools/         # Agent callable filesystem tools
│   └── use_cases/     # Application workflows (chat, model selection)
├── Cargo.toml         # Rust package metadata and dependencies
├── Cargo.lock         # Resolved dependency versions
└── README.md          # This file
```

## Key Components

### Command-Line Interface

The CLI entry point is defined in `src/cmd/cli.rs`. It provides:

- **Base URL**: Required OpenAI-compatible API base URL; Rigel calls `/models` and `/chat/completions` below it
- **API Key**: Optional bearer authentication for API endpoints

```bash
cargo run -- -b <api-base-url> -k <api-key>
```

### Agent Controller

Located in `src/controllers/index_controller.rs`, the IndexController orchestrates agent operations:

- Establishes connection to an OpenAI-compatible server
- Selects available models deterministically
- Initializes chat with system prompt
- Manages conversation turns (default: 12)
- Routes filesystem tools to appropriate agents

### Filesystem Tools

Rigel provides a suite of atomic filesystem operations, each with narrow responsibilities:

| Tool               | Description                                     |
| ------------------ | ----------------------------------------------- |
| `create_file`      | Create new files in workspace                   |
| `read_file`        | Read file content (returns SHA-256 revision)    |
| `apply_patch`      | Atomic UTF-8 text edits with revision checking  |
| `create_directory` | Create directories recursively                  |
| `delete_directory` | Remove directory and contents                   |
| `delete_file`      | Delete existing file (requires existence check) |
| `find_paths`       | Recursive glob pattern matching                 |
| `list_directory`   | Inspect directory contents                      |
| `move_path`        | Rename or move files/directories                |
| `search_text`      | Search file contents (supports regex)           |
| `stat`             | Return file/directory metadata                  |

**Security Constraints**: All tools work only within the workspace where Rigel started. Workspace-relative paths are validated; absolute paths and parent references (`..`) are rejected. The workspace root is protected from delete/move operations.

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
