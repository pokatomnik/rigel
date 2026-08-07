use std::{collections::HashMap, error::Error};

use anyhow::{Context, Result, bail};
use rig::tool::{rmcp::McpClientHandler, server::ToolServerHandle};
use rmcp::{
    model::{ClientInfo, Tool},
    service::{RoleClient, RunningService, ServerSink},
    transport::{IntoTransport, StreamableHttpClientTransport, TokioChildProcess},
};
use serde::Deserialize;
use tokio::process::Command;

use rig::tool::server::ToolServer;

pub(crate) struct McpRegistry {
    connections: Vec<McpConnection>,
}

struct McpConnection {
    service: RunningService<RoleClient, McpClientHandler>,
    tools: Vec<Tool>,
}

#[derive(Deserialize)]
struct McpConfig {
    #[serde(rename = "mcpServers")]
    servers: HashMap<String, ServerConfig>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ServerConfig {
    Stdio(StdioConfig),
    Http(HttpConfig),
}

#[derive(Deserialize)]
struct StdioConfig {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

#[derive(Deserialize)]
struct HttpConfig {
    url: String,
}

impl McpRegistry {
    pub async fn from_config_file(path: &str) -> Result<Self> {
        let content = tokio::fs::read_to_string(path)
            .await
            .with_context(|| format!("failed to read MCP config `{path}`"))?;
        let config = parse_config(&content)
            .with_context(|| format!("failed to parse MCP config `{path}`"))?;
        let tool_server = ToolServer::new().run();
        let mut connections = Vec::with_capacity(config.servers.len());

        for (name, server) in config.servers {
            connections.push(connect(name.as_str(), server, tool_server.clone()).await?);
        }

        Ok(Self { connections })
    }

    pub fn empty() -> Self {
        Self {
            connections: Vec::new(),
        }
    }

    pub fn tools(&self) -> Vec<(Vec<Tool>, ServerSink)> {
        self.connections
            .iter()
            .map(|connection| (connection.tools.clone(), connection.service.peer().clone()))
            .collect()
    }
}

async fn connect(
    name: &str,
    config: ServerConfig,
    tool_server: ToolServerHandle,
) -> Result<McpConnection> {
    match config {
        ServerConfig::Stdio(config) => connect_stdio(name, config, tool_server).await,
        ServerConfig::Http(config) => connect_http(name, config, tool_server).await,
    }
}

async fn connect_stdio(
    name: &str,
    config: StdioConfig,
    tool_server: ToolServerHandle,
) -> Result<McpConnection> {
    let mut command = Command::new(config.command);
    command.args(config.args).envs(config.env);
    let transport = TokioChildProcess::new(command)
        .with_context(|| format!("failed to start MCP server `{name}`"))?;
    connect_transport(name, transport, tool_server).await
}

async fn connect_http(
    name: &str,
    config: HttpConfig,
    tool_server: ToolServerHandle,
) -> Result<McpConnection> {
    if config.url.trim().is_empty() {
        bail!("MCP server `{name}` has an empty URL");
    }
    let transport = StreamableHttpClientTransport::from_uri(config.url);
    connect_transport(name, transport, tool_server).await
}

async fn connect_transport<T, E, A>(
    name: &str,
    transport: T,
    tool_server: ToolServerHandle,
) -> Result<McpConnection>
where
    T: IntoTransport<RoleClient, E, A>,
    E: Error + Send + Sync + 'static,
{
    let handler = McpClientHandler::new(ClientInfo::default(), tool_server);
    let service = handler
        .connect(transport)
        .await
        .with_context(|| format!("failed to connect MCP server `{name}`"))?;
    let tools = service
        .list_tools(Default::default())
        .await
        .with_context(|| format!("failed to list tools from MCP server `{name}`"))?
        .tools;
    Ok(McpConnection { service, tools })
}

fn parse_config(content: &str) -> Result<McpConfig> {
    serde_json::from_str(content).context("invalid MCP configuration JSON")
}

#[cfg(test)]
mod tests {
    use super::{ServerConfig, parse_config};

    #[test]
    fn parses_unlimited_mixed_servers() -> anyhow::Result<()> {
        let config = parse_config(
            r#"{
                "mcpServers": {
                    "local": { "command": "node", "args": ["server.js"] },
                    "remote": { "url": "https://example.com/mcp" }
                }
            }"#,
        )?;

        assert_eq!(config.servers.len(), 2);
        assert!(matches!(config.servers["local"], ServerConfig::Stdio(_)));
        assert!(matches!(config.servers["remote"], ServerConfig::Http(_)));
        Ok(())
    }

    #[test]
    fn rejects_unknown_transport_shape() {
        let result = parse_config(r#"{ "mcpServers": { "bad": { "port": 42 } } }"#);
        assert!(result.is_err());
    }
}
