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
use tokio::sync::Mutex;

use crate::shared::config::rigel_config::RigelConfig;
use crate::shared::tool_permissions::catalog::ToolPermissionCatalog;

use super::http_config::HttpConfig;
use super::server_config::ServerConfig;
use super::stdio_config::StdioConfig;

pub(crate) struct McpRegistry {
    config: Arc<RigelConfig>,
    connections: Vec<McpConnection>,
    selected_indices: Mutex<Option<Vec<usize>>>,
}

struct McpConnection {
    name: String,
    service: RunningService<RoleClient, McpClientHandler>,
    tools: Vec<Tool>,
}

impl McpRegistry {
    pub async fn from_config(config: Arc<RigelConfig>) -> anyhow::Result<Self> {
        let tool_server = ToolServer::new().run();
        let mut connections = Vec::with_capacity(config.mcp_servers().len());

        for (name, server) in config.mcp_servers() {
            connections.push(server.connect(name.as_str(), tool_server.clone()).await?);
        }

        Ok(Self {
            config,
            connections,
            selected_indices: Mutex::new(None),
        })
    }

    pub(crate) fn config(&self) -> Arc<RigelConfig> {
        self.config.clone()
    }

    #[allow(dead_code)]
    pub fn tools(&self) -> Vec<(Vec<Tool>, ServerSink)> {
        self.connections
            .iter()
            .map(|connection| (connection.tools.clone(), connection.service.peer().clone()))
            .collect()
    }

    pub async fn select_tools(&self) -> Vec<(Vec<Tool>, ServerSink)> {
        if let Some(selected_indices) = self.selected_indices.lock().await.clone() {
            return self.tools_for(selected_indices.as_slice());
        }

        self.reselect_tools().await
    }

    pub async fn reselect_tools(&self) -> Vec<(Vec<Tool>, ServerSink)> {
        let selected_indices = self.choose_indices();
        *self.selected_indices.lock().await = Some(selected_indices.clone());
        self.tools_for(selected_indices.as_slice())
    }

    fn choose_indices(&self) -> Vec<usize> {
        let connection_names = self
            .connections
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<&str>>();
        let selected_names = connection_names
            .clone()
            .iter()
            .map(|v| (v.to_string(), true))
            .collect::<Vec<(String, bool)>>();
        dialoguer::MultiSelect::new()
            .with_prompt("Select MCP servers")
            .report(false)
            .clear(true)
            .items_checked(selected_names)
            .interact()
            .unwrap_or_default()
    }

    fn tools_for(&self, selected_indices: &[usize]) -> Vec<(Vec<Tool>, ServerSink)> {
        self.connections
            .iter()
            .enumerate()
            .filter(|(index, _)| selected_indices.contains(index))
            .map(|(_, connection)| (connection.tools.clone(), connection.service.peer().clone()))
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
        Ok(McpConnection {
            service,
            tools,
            name: name.to_string(),
        })
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
        let (transport, _stderr) = TokioChildProcess::builder(command)
            // TODO redirect to mcp.log file
            .stderr(Stdio::null())
            .spawn()
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
