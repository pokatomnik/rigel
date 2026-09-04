# Tool permissions

This module decides whether a tool call may execute. The decision is made at
the tool execution boundary, so a denial does not require a special model
response or extra instructions in the system prompt.

Interaction graph:

```text
ChatController / main
    └─ pass RigelConfig and layer paths to AgentDependencies
       └─ Agent::build_agent
          ├─ registers built-in and MCP tools in ToolPermissionCatalog
          └─ adds ToolPermissionHook to the agent used by Chat
             └─ ToolPermissionManager::decide before tool execution
                ├─ reads project/global policies through RigelConfig
                ├─ invokes TerminalIO confirmation when no policy exists
                └─ returns run or a model-visible skip to Chat/the model
```

`ToolPermissionCatalog` records whether confirmation is required: regular
built-in tools may be automatic, while MCP tools and unknown names require
confirmation. `ToolPermissionHook` receives the actual tool name from Rig,
looks up its requirement in the catalog, and passes the request to the manager.
`Chat::run` remains a linear loop and knows nothing about policy lookup or
confirmation; a denial reaches it through the agent call result.

`ToolPermissionManager` checks decisions in the order session, project, global.
A session allowance exists only in the current agent's memory. `Allow once`
does not persist anything. `Allow for session` is kept in memory. `Allow for
project` is written to the project config, while `Allow forever` is written to
the global config through `RigelConfig::update_policy_file`; existing TOML
values are preserved. An explicit denial and a policy with `allow = false`
only block the current call and are not written back to the config.

`RigelConfig::policy_from_paths` reads the configuration layers for every
decision, so policy changes made between calls take effect without rebuilding
the chat. Project policies take precedence over global policies.
