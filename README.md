# Rigel

Rigel is a cozy LLM agent implemented as a Rust 2024 CLI, designed for chatting with models served by Ollama. It features a robust tool-calling architecture and advanced error recovery mechanisms to ensure reliable interactions with language models.

## Key Features

- **Ollama Integration**: Seamlessly chat with local LLMs via the Ollama API.
- **Filesystem Toolset**: A comprehensive set of agent-callable tools for workspace manipulation:
    - `list_directory`: Inspect directory contents.
    - `find_paths`: Recursive path searching with glob patterns.
    - `search_text`: Search within files and directories.
    - `stat`: Retrieve file metadata.
    - `read_file`: Read UTF-8 files with revision tracking (SHA-256).
    - `apply_patch`: Atomic, revision-checked file updates to prevent data loss and race conditions.
    - `create_file`, `create_directory`, `move_path`, `delete_file`, `delete_directory`.
- **Smart Error Recovery**: Designed to handle weak models by providing intelligible recovery steps and managing failure budgets to prevent infinite loops.
- **Safety First**: Strict workspace boundaries prevent tools from accessing files outside the project root.

## Getting Started

### Prerequisites

- Rust (Edition 2024)
- [Ollama](https://ollama.com/) installed and running locally.

### Installation

Clone the repository and build the project:

```bash
cargo build
```

### Running Rigel

To start Rigel against a local Ollama server:

```bash
cargo run -- --base-url http://localhost:11434
```

For more options, use the help command:

```bash
cargo run -- --help
```

## Project Structure

- `src/cmd/`: Command-line interface parsing.
- `src/controllers/`: Request orchestration and control flow.
- `src/use_cases/`: Application workflows (e.g., chat and model selection).
- `src/tools/`: Implementation of agent-callable tools.
- `src/prompts/`: LLM prompts and template text.
- `src/shared/`: Common utilities and terminal I/O.
- `src/entities/`: Core data models.

## Development

### Testing

Run the complete test suite:

```bash
cargo test --all-targets
```

### Linting and Formatting

Verify formatting:
```bash
cargo fmt --all -- --check
```

Run Clippy for static analysis:
```bash
cargo clippy --all-targets --all-features
```
