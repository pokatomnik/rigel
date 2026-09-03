---
name: configure-rigel
description: Configure Rigel with config.toml, MCP servers, and project or global skills.
---

# Configure Rigel

## Activation behavior

This skill is reference context, not a request to summarize or recite the
manifest. When the user activates it, do not summarize these instructions.
Use them to answer the user's later configuration questions.

After activation, proactively ask: “How can I help you configure Rigel?” Use an
equivalent natural question in the language of the user's latest messages when
that language is known. If the user's language is not known yet, ask the
question in English. Keep this activation response concise and wait for the
user's configuration request.

## Configuration entry points

Rigel reads the default configuration from `~/.rigel/config.toml` when no
profile is supplied. Run `rigel init` to create or overwrite that file. Use
`rigel init --base-url <URL>` to set the OpenAI-compatible API base URL and
`rigel init --api-key-env <NAME>` to tell Rigel which environment variable
contains the API key. The key itself is not stored in TOML.

Use `rigel chat --profile <PATH>` (or `-p <PATH>`) to load a specific TOML
file. Errors loading an explicit profile are returned to the caller. Errors
loading the default configuration fall back to Rigel's defaults.

## TOML fields

`baseUrl` defaults to `http://127.0.0.1:1234/v1`:

```toml
baseUrl = "http://127.0.0.1:1234/v1"
envKey = "OPENAI_API_KEY"

[models."exact-model-id".params]
reasoning_effort = "high"

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

`envKey` is the name of the environment variable from which Rigel reads the
API key. Keep the secret in the environment rather than in `config.toml`.

## Model parameters

The key under `models` must exactly match the model ID, including letter case.
The `params` table is forwarded as additional top-level JSON request fields.
Values can be TOML strings, booleans, numbers, arrays, or nested tables. Rigel
does not normalize provider-specific parameters; the provider defines their
meaning.

Do not put `model`, `messages`, `tools`, `tool_choice`, `temperature`,
`max_tokens`, or `stream` in `params`. These fields belong to Rigel's request
transport and are rejected when configured there.

## MCP servers

An `stdio` server starts a local process. `command` is required; `args` and
`env` are optional. An `http` server uses the required `url` field. Rigel
connects to every configured MCP server and lists its tools while creating
`McpRegistry` at startup, before the chat begins. MCP tools are then available
to the agent according to the server selection flow.

## Adding skills

Add a skill as a direct child of either supported directory:

```text
<project-root>/.agents/skills/<skill-name>/SKILL.md
~/.agents/skills/<skill-name>/SKILL.md
```

The project directory is the current working directory from which Rigel was
started. Discovery is not recursive and does not search parent directories or
find a Git root. Each skill must be a direct child directory with a regular
`SKILL.md` file. The global directory is skipped if the user home directory is
unavailable.

## Using skills

Enter `/skills` in the interactive chat to see the available skills and choose
one from the interactive selector. `/skill` is a compatible alias for
`/skills`. Only the skill selected by the user is sent to the agent; discovered
skills are not all added automatically to every request. The built-in
`configure-rigel` skill is always available and is compiled into Rigel.
