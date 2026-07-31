# Rigel

A Rust 2024 CLI agent for interacting with LLM models served by Ollama.

## Project Overview

Rigel is a conversational LLM agent that integrates filesystem tools into an interactive CLI session. It connects to Ollama servers and provides atomic, deterministic filesystem operations within the workspace context. All tools work only within the workspace where Rigel started - absolute paths and parent references are rejected for security reasons.

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
# Connect to Ollama server with base URL
cargo run -- -b <ollama-url>

# Optionally provide API key for authenticated endpoints
cargo run -- -b <ollama-url> -k <api-key>

# Start Rigel with local Ollama server (default: http://localhost:11434)
cargo run -- -b http://localhost:11434

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

# Start Rigel with local Ollama server
cargo run -- --base-url http://localhost:11434

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

- **Base URL**: Connect to Ollama server at a configurable endpoint (default: http://localhost:11434)
- **API Key**: Optional authentication for Ollama endpoints

```bash
cargo run -- -b <ollama-url> -k <api-key>
```

### Agent Controller

Located in `src/controllers/index_controller.rs`, the IndexController orchestrates agent operations:

- Establishes connection to Ollama server
- Selects available models deterministically
- Initializes chat with system prompt
- Manages conversation turns (default: 12)
- Routes filesystem tools to appropriate agents

### Filesystem Tools

Rigel provides a suite of atomic filesystem operations, each with narrow responsibilities:

| Tool | Description |
|------|-------------|
| `create_file` | Create new files in workspace |
| `read_file` | Read file content (returns SHA-256 revision) |
| `apply_patch` | Atomic UTF-8 text edits with revision checking |
| `create_directory` | Create directories recursively |
| `delete_directory` | Remove directory and contents |
| `delete_file` | Delete existing file (requires existence check) |
| `find_paths` | Recursive glob pattern matching |
| `list_directory` | Inspect directory contents |
| `move_path` | Rename or move files/directories |
| `search_text` | Search file contents (supports regex) |
| `stat` | Return file/directory metadata |

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
