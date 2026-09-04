use std::sync::Arc;

use reqwest::Client;
use rig::{Agent as RigAgent, completion::CompletionModel as CompletionModelTrait};

use super::super::{
    config::rigel_config::ToolPermissionPaths, goal::goal_state::GoalState,
    mcp_registry::registry::McpRegistry, terminal::terminal_io::TerminalIO,
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
    tool_permission_paths: Option<ToolPermissionPaths>,
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
            tool_permission_paths: None,
        }
    }

    pub(crate) fn with_goal_state(mut self, goal_state: Arc<GoalState>) -> Self {
        self.goal_state = Some(goal_state);
        self
    }

    pub(crate) fn with_tool_permission_paths(mut self, paths: ToolPermissionPaths) -> Self {
        self.tool_permission_paths = Some(paths);
        self
    }

    pub(crate) fn goal_state(&self) -> anyhow::Result<Arc<GoalState>> {
        self.goal_state
            .clone()
            .ok_or_else(|| anyhow::anyhow!("goal state was not configured for the chat agent"))
    }

    pub(super) fn tool_permission_paths(&self) -> anyhow::Result<ToolPermissionPaths> {
        self.tool_permission_paths.clone().ok_or_else(|| {
            anyhow::anyhow!("tool permission configuration paths were not configured")
        })
    }
}
