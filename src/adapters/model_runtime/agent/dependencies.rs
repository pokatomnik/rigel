use std::sync::Arc;

use reqwest::Client;
use rig::{Agent as RigAgent, completion::CompletionModel as CompletionModelTrait};

use crate::{
    adapters::{
        mcp_registry::registry::McpRegistry, terminal::terminal_io::TerminalIO,
        workspace::filesystem::WorkspaceCapabilities,
    },
    contracts::tools::goal::GoalCompleter,
    entities::{configuration::paths::ConfigurationPaths, goal::goal_state::GoalState},
};

pub(crate) struct ConfiguredAgent<M: CompletionModelTrait> {
    pub(crate) agent: RigAgent<M>,
    pub(crate) max_context_tokens: Option<u64>,
}

impl<M: CompletionModelTrait> ConfiguredAgent<M> {
    #[cfg(test)]
    pub(crate) fn from_agent(agent: RigAgent<M>) -> Self {
        Self {
            agent,
            max_context_tokens: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct AgentDependencies {
    pub(super) terminal_io: Arc<TerminalIO>,
    pub(super) http_client: Arc<Client>,
    pub(super) mcp_registry: Arc<McpRegistry>,
    goal_state: Option<Arc<GoalState>>,
    goal_completer: Option<Arc<dyn GoalCompleter>>,
    tool_permission_paths: Option<ConfigurationPaths>,
    workspace: Option<Arc<WorkspaceCapabilities>>,
}

impl AgentDependencies {
    pub(crate) fn new(
        terminal_io: Arc<TerminalIO>,
        http_client: Arc<Client>,
        mcp_registry: Arc<McpRegistry>,
    ) -> Self {
        Self {
            terminal_io,
            http_client,
            mcp_registry,
            goal_state: None,
            goal_completer: None,
            tool_permission_paths: None,
            workspace: None,
        }
    }

    pub(crate) fn with_goal_state(mut self, goal_state: Arc<GoalState>) -> Self {
        self.goal_state = Some(goal_state);
        self
    }

    /// Supplies the contract used by the goal-completion tool and hook.
    pub(crate) fn with_goal_completer(mut self, completer: Arc<dyn GoalCompleter>) -> Self {
        self.goal_completer = Some(completer);
        self
    }

    pub(crate) fn with_tool_permission_paths(mut self, paths: ConfigurationPaths) -> Self {
        self.tool_permission_paths = Some(paths);
        self
    }

    pub(crate) fn with_workspace(mut self, workspace: Arc<WorkspaceCapabilities>) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Returns the terminal adapter used by runtime-owned tools and sessions.
    pub(crate) fn terminal_io(&self) -> Arc<TerminalIO> {
        self.terminal_io.clone()
    }

    /// Refreshes selected MCP tools before a replacement session is built.
    pub(crate) async fn reselect_mcp(&self) {
        self.mcp_registry.reselect_tools().await;
    }

    pub(crate) fn workspace(&self) -> anyhow::Result<Arc<WorkspaceCapabilities>> {
        self.workspace
            .clone()
            .ok_or_else(|| anyhow::anyhow!("workspace capabilities were not configured"))
    }

    pub(crate) fn goal_state(&self) -> anyhow::Result<Arc<GoalState>> {
        self.goal_state
            .clone()
            .ok_or_else(|| anyhow::anyhow!("goal state was not configured for the chat agent"))
    }

    /// Returns the root-composed goal capability for tool registration.
    pub(super) fn goal_completer(&self) -> anyhow::Result<Arc<dyn GoalCompleter>> {
        self.goal_completer
            .clone()
            .ok_or_else(|| anyhow::anyhow!("goal completer was not configured for the chat agent"))
    }

    pub(super) fn tool_permission_paths(&self) -> anyhow::Result<ConfigurationPaths> {
        self.tool_permission_paths.clone().ok_or_else(|| {
            anyhow::anyhow!("tool permission configuration paths were not configured")
        })
    }
}
