use std::error::Error;

use anyhow::Context;
use rig::AgentBuilder;
use rig::agent::WithBuilderTools;
use rig::completion::CompletionModel;
use rig::tool::server::ToolServer;
use rig::tool::{rmcp::McpClientHandler, server::ToolServerHandle};
use rmcp::{
    model::{ClientInfo, Tool},
    service::{RoleClient, RunningService, ServerSink},
    transport::{IntoTransport, StreamableHttpClientTransport, TokioChildProcess},
};
use tokio::process::Command;

use crate::shared::rigel_config::RigelConfig;

use super::http_config::HttpConfig;
use super::server_config::ServerConfig;
use super::stdio_config::StdioConfig;

#[derive(Default)]
pub(crate) struct McpRegistry {
    connections: Vec<McpConnection>,
}

struct McpConnection {
    service: RunningService<RoleClient, McpClientHandler>,
    tools: Vec<Tool>,
}

impl McpRegistry {
    pub async fn from_config(config: &RigelConfig) -> anyhow::Result<Self> {
        let tool_server = ToolServer::new().run();
        let mut connections = Vec::with_capacity(config.servers.len());

        for (name, server) in &config.servers {
            connections.push(server.connect(name.as_str(), tool_server.clone()).await?);
        }

        Ok(Self { connections })
    }

    pub fn tools(&self) -> Vec<(Vec<Tool>, ServerSink)> {
        self.connections
            .iter()
            .map(|connection| (connection.tools.clone(), connection.service.peer().clone()))
            .collect()
    }
}

trait McpConnector: Send + Sync {
    async fn connect(
        &self,
        name: &str,
        tool_server: ToolServerHandle,
    ) -> anyhow::Result<McpConnection>;

    async fn connect_transport<T, E, A>(
        name: &str,
        transport: T,
        tool_server: ToolServerHandle,
    ) -> anyhow::Result<McpConnection>
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
}

impl McpConnector for StdioConfig {
    async fn connect(
        &self,
        name: &str,
        tool_server: ToolServerHandle,
    ) -> anyhow::Result<McpConnection> {
        let mut command = Command::new(&self.command);
        command.args(&self.args).envs(&self.env);
        let transport = TokioChildProcess::new(command)
            .with_context(|| format!("failed to start MCP server `{name}`"))?;
        Self::connect_transport(name, transport, tool_server).await
    }
}

impl McpConnector for HttpConfig {
    async fn connect(
        &self,
        name: &str,
        tool_server: ToolServerHandle,
    ) -> anyhow::Result<McpConnection> {
        if self.url.trim().is_empty() {
            anyhow::bail!("MCP server `{name}` has an empty URL");
        }
        let transport = StreamableHttpClientTransport::from_uri(self.url.as_str());
        Self::connect_transport(name, transport, tool_server).await
    }
}

impl McpConnector for ServerConfig {
    async fn connect(
        &self,
        name: &str,
        tool_server: ToolServerHandle,
    ) -> anyhow::Result<McpConnection> {
        match self {
            ServerConfig::Stdio(config) => config.connect(name, tool_server).await,
            ServerConfig::Http(config) => config.connect(name, tool_server).await,
        }
    }
}

pub(crate) trait McpToolsExt<M: CompletionModel> {
    fn mcp_tools(self, servers: &[(Vec<Tool>, ServerSink)]) -> AgentBuilder<M, WithBuilderTools>;
}

impl<M: CompletionModel> McpToolsExt<M> for AgentBuilder<M, WithBuilderTools> {
    fn mcp_tools(
        mut self,
        servers: &[(Vec<Tool>, ServerSink)],
    ) -> AgentBuilder<M, WithBuilderTools> {
        for (tools, peer) in servers {
            self = self.rmcp_tools(tools.clone(), peer.clone());
        }
        self
    }
}
