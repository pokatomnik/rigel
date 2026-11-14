use std::error::Error;
use std::process::Stdio;
use std::sync::Arc;

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

use crate::{
    controllers::model_runtime::permissions::catalog::ToolPermissionCatalog,
    entities::{
        agent_event::status::{McpServerState, McpServerStatus},
        configuration::{
            config::RigelConfig,
            server_config::{McpHttpConfig, McpServerConfig, McpStdioConfig},
        },
    },
};

pub(crate) struct McpRegistry {
    connections: Vec<McpConnection>,
    statuses: Vec<McpServerStatus>,
}

struct McpConnection {
    service: RunningService<RoleClient, McpClientHandler>,
    tools: Vec<Tool>,
}

impl McpRegistry {
    /// Connects every configured server and keeps failed servers as visible statuses.
    ///
    /// A failed optional server never blocks the chat: its tools are absent and the UI shows a
    /// red status. Successfully connected servers keep working for the whole session.
    pub async fn from_config(config: Arc<RigelConfig>) -> Self {
        let tool_server = ToolServer::new().run();
        let mut names = config
            .mcp_servers()
            .keys()
            .cloned()
            .collect::<Vec<String>>();
        names.sort();
        let mut connections = Vec::with_capacity(names.len());
        let mut statuses = Vec::with_capacity(names.len());
        for name in names {
            let Some(server) = config.mcp_servers().get(&name) else {
                continue;
            };
            match server.connect(name.as_str(), tool_server.clone()).await {
                Ok(connection) => {
                    connections.push(connection);
                    statuses.push(McpServerStatus::new(name, McpServerState::Running));
                }
                Err(_) => statuses.push(McpServerStatus::new(name, McpServerState::Failed)),
            }
        }
        Self {
            connections,
            statuses,
        }
    }

    /// Returns the configured server states for the header and palette.
    pub(crate) fn statuses(&self) -> Vec<McpServerStatus> {
        self.statuses.clone()
    }

    /// Returns tools from every server that connected successfully.
    pub async fn select_tools(&self) -> Vec<(Vec<Tool>, ServerSink)> {
        self.tools()
    }

    /// Rebuilds the tool list after a session replacement.
    pub async fn reselect_tools(&self) -> Vec<(Vec<Tool>, ServerSink)> {
        self.tools()
    }

    fn tools(&self) -> Vec<(Vec<Tool>, ServerSink)> {
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

impl McpConnector for McpStdioConfig {
    async fn connect(
        &self,
        name: &str,
        tool_server: ToolServerHandle,
    ) -> anyhow::Result<McpConnection> {
        let Some(command_name) = self.command.as_deref() else {
            anyhow::bail!("MCP server `{name}` has no command");
        };
        let mut command = Command::new(command_name);
        command.args(&self.args).envs(&self.env);
        let (transport, _stderr) = TokioChildProcess::builder(command)
            // TODO redirect to mcp.log file
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("failed to start MCP server `{name}`"))?;
        Self::connect_transport(name, transport, tool_server).await
    }
}

impl McpConnector for McpHttpConfig {
    async fn connect(
        &self,
        name: &str,
        tool_server: ToolServerHandle,
    ) -> anyhow::Result<McpConnection> {
        let Some(url) = self.url.as_deref() else {
            anyhow::bail!("MCP server `{name}` has no URL");
        };
        if url.trim().is_empty() {
            anyhow::bail!("MCP server `{name}` has an empty URL");
        }
        let transport = StreamableHttpClientTransport::from_uri(url);
        Self::connect_transport(name, transport, tool_server).await
    }
}

impl McpConnector for McpServerConfig {
    async fn connect(
        &self,
        name: &str,
        tool_server: ToolServerHandle,
    ) -> anyhow::Result<McpConnection> {
        match self {
            McpServerConfig::Stdio(config) => config.connect(name, tool_server).await,
            McpServerConfig::Http(config) => config.connect(name, tool_server).await,
        }
    }
}

pub(crate) trait McpToolsExt<M: CompletionModel> {
    fn mcp_tools(
        self,
        servers: &[(Vec<Tool>, ServerSink)],
        catalog: &mut ToolPermissionCatalog,
    ) -> AgentBuilder<M, WithBuilderTools>;
}

impl<M: CompletionModel> McpToolsExt<M> for AgentBuilder<M, WithBuilderTools> {
    fn mcp_tools(
        mut self,
        servers: &[(Vec<Tool>, ServerSink)],
        catalog: &mut ToolPermissionCatalog,
    ) -> AgentBuilder<M, WithBuilderTools> {
        for (tools, peer) in servers {
            for tool in tools {
                catalog.register_mcp_tool(tool.name.as_ref());
            }
            self = self.rmcp_tools(tools.clone(), peer.clone());
        }
        self
    }
}
