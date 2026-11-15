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
profile is supplied. Run `rigel init` to create that file without overwriting an
existing one. Use `rigel init --base-url <URL>` to set the OpenAI-compatible API
base URL and `rigel init --api-key-env <NAME>` to tell Rigel which environment
variable contains the API key. Rigel never stores the API key in the
configuration file; it reads the named environment variable at startup.

Use `rigel chat --profile <PATH>` (or `-p <PATH>`) to load a specific TOML
file as the global layer. The project layer at
`<current-dir>/.rigel/config.toml` is still read and overrides it. Read or TOML
parse errors in the optional default-global and project files are ignored; Rigel
uses the other valid layer, or defaults if neither layer is usable. An explicit
profile is required, so its read or parse errors are returned to the caller;
the project layer remains optional.

## TOML fields

`baseUrl` defaults to `http://127.0.0.1:1234/v1`:

```toml
baseUrl = "http://127.0.0.1:1234/v1"
envKey = "OPENAI_API_KEY"

[models."exact-model-id"]
context_size = 128000

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

[policies."run_command"]
allow = true
```

`envKey` names the environment variable that contains the provider key. Rigel
never stores the key in `config.toml`; export the named variable before starting
Rigel.

`context_size` is required for every selectable model and is expressed in
tokens. Rigel does not query `/models` and does not derive a context size from
the provider. Models without `context_size` are hidden from the model palette;
if no configured model has it, Rigel shows that it is not configured and offers
only exit. There is no default or fallback context size.

## Tool permissions

Policies are keyed by the runtime tool name. The project policy takes
precedence over the global policy, and `allow = false` is an explicit denial:

```toml
[policies."run_command"]
allow = false
```

Without a policy, automatic tools run without confirmation while other tools
ask. The confirmation choices are deny, allow once, allow for project, allow
for session, and allow forever. Project choices are written to
`<current-dir>/.rigel/config.toml`; forever choices are written to the
`--profile` file or to `~/.rigel/config.toml` when no profile is supplied.
Session choices last until the current agent is rebuilt. Deny and allow-once
choices are never saved. `rigel init` does not add policies.

## Model parameters

The key under `models` must exactly match the model ID used by the provider,
including letter case. Every entry needs `context_size` to appear in the model
palette:

```toml
[models."exact-model-id"]
context_size = 128000

[models."exact-model-id".params]
reasoning_effort = "high"
```

The `params` table is forwarded as additional top-level JSON request fields.
Values can be TOML strings, booleans, numbers, arrays, or nested tables. Rigel
does not normalize provider-specific parameters; the provider defines their
meaning. A zero `context_size` is rejected when the configuration is loaded.

Do not put `model`, `messages`, `tools`, `tool_choice`, `temperature`,
`max_tokens`, or `stream` in `params`. These fields belong to Rigel's request
transport and are rejected when configured there.

## MCP servers

An `stdio` server starts a local process. `command` is required; `args` and
`env` are optional. An `http` server uses the required `url` field. Rigel
connects to every configured MCP server at startup. Servers that fail to start
are skipped, their tools are absent, and their status is shown in the header
and the MCP-server palette level. Successfully connected servers keep working
for the session; there is no server-selection flow.

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

Open the command palette in the interactive chat (`Ctrl+K`, or `/` in an empty
composer), choose **Use a skill…**, and select one from the list. Only the skill
selected by the user is sent to the agent; discovered skills are not all added
automatically to every request. The built-in `configure-rigel` skill is always
available and is compiled into Rigel.
