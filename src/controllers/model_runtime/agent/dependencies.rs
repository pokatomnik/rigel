use std::sync::Arc;

use reqwest::Client;
use rig::{Agent as RigAgent, completion::CompletionModel as CompletionModelTrait};

use super::agent_tool_set::RegisteredToolSet;
use crate::{
    controllers::{mcp_registry::registry::McpRegistry, terminal::terminal_io::TerminalIO},
    entities::selected_model::selection::SelectedModel,
    infrastructure::{
        conversation::ContextCompaction,
        model::ModelSelector,
        prompts::PromptProvider,
        recovery::{ToolRecoveryPolicyFactory, TurnRecovery},
        tools::{goal::GoalCompleter, permissions::ToolPermissionEvaluator},
    },
};

pub(crate) struct ConfiguredAgent<M: CompletionModelTrait> {
    /// The provider agent configured with the selected tools and runtime hooks.
    pub(crate) agent: RigAgent<M>,
    /// The selected model inherited by tools and child sessions.
    pub(crate) selected_model: Option<SelectedModel>,
    /// The context limit used by the session's compaction policy, if known.
    pub(crate) max_context_tokens: Option<u64>,
}

impl<M: CompletionModelTrait> ConfiguredAgent<M> {
    /// Wraps a test agent without assigning a provider context limit.
    #[cfg(test)]
    pub(crate) fn from_agent(agent: RigAgent<M>) -> Self {
        Self {
            agent,
            selected_model: None,
            max_context_tokens: None,
        }
    }
}

#[derive(Clone)]
pub(crate) struct AgentWiring {
    pub(super) terminal_io: Arc<TerminalIO>,
    pub(super) http_client: Arc<Client>,
    pub(super) mcp_registry: Arc<McpRegistry>,
    goal_completer: Option<Arc<dyn GoalCompleter>>,
    prompt_provider: Option<Arc<dyn PromptProvider>>,
    model_selector: Option<Arc<dyn ModelSelector>>,
    permission_manager: Option<Arc<dyn Fn() -> Arc<dyn ToolPermissionEvaluator> + Send + Sync>>,
    recovery_policy_factory: Option<Arc<dyn ToolRecoveryPolicyFactory>>,
    turn_recovery: Option<Arc<dyn TurnRecovery>>,
    context_compaction: Option<Arc<dyn ContextCompaction>>,
    chat_tools: Option<Arc<RegisteredToolSet>>,
    subagent_tools: Option<Arc<RegisteredToolSet>>,
}

impl AgentWiring {
    /// Creates runtime wiring from provider and terminal boundary implementations.
    pub(crate) fn new(
        terminal_io: Arc<TerminalIO>,
        http_client: Arc<Client>,
        mcp_registry: Arc<McpRegistry>,
    ) -> Self {
        Self {
            terminal_io,
            http_client,
            mcp_registry,
            goal_completer: None,
            prompt_provider: None,
            model_selector: None,
            permission_manager: None,
            recovery_policy_factory: None,
            turn_recovery: None,
            context_compaction: None,
            chat_tools: None,
            subagent_tools: None,
        }
    }

    /// Supplies the goal capability used by chat sessions and their Rig hook.
    pub(crate) fn with_goal_completer(mut self, completer: Arc<dyn GoalCompleter>) -> Self {
        self.goal_completer = Some(completer);
        self
    }

    /// Supplies the prompt capability used to configure and compact sessions.
    pub(crate) fn with_prompt_provider(mut self, provider: Arc<dyn PromptProvider>) -> Self {
        self.prompt_provider = Some(provider);
        self
    }

    /// Supplies the model presentation capability used for interactive selection.
    pub(crate) fn with_model_selector(mut self, selector: Arc<dyn ModelSelector>) -> Self {
        self.model_selector = Some(selector);
        self
    }

    /// Supplies a fresh permission evaluator factory for each rebuilt session.
    pub(crate) fn with_permission_manager(
        mut self,
        factory: Arc<dyn Fn() -> Arc<dyn ToolPermissionEvaluator> + Send + Sync>,
    ) -> Self {
        self.permission_manager = Some(factory);
        self
    }

    /// Supplies the run-local policy factory used by each provider recovery hook.
    pub(crate) fn with_recovery_policy_factory(
        mut self,
        factory: Arc<dyn ToolRecoveryPolicyFactory>,
    ) -> Self {
        self.recovery_policy_factory = Some(factory);
        self
    }

    /// Supplies the turn-recovery workflow shared by chat and subagent sessions.
    pub(crate) fn with_turn_recovery(mut self, recovery: Arc<dyn TurnRecovery>) -> Self {
        self.turn_recovery = Some(recovery);
        self
    }

    /// Supplies provider-neutral compaction policy for sessions built from this wiring.
    pub(crate) fn with_context_compaction(mut self, policy: Arc<dyn ContextCompaction>) -> Self {
        self.context_compaction = Some(policy);
        self
    }

    /// Returns the configured context-compaction policy or reports missing composition.
    pub(crate) fn context_compaction(&self) -> anyhow::Result<Arc<dyn ContextCompaction>> {
        self.context_compaction
            .clone()
            .ok_or_else(|| anyhow::anyhow!("context compaction policy was not configured"))
    }

    /// Installs complete immutable tool sets before the wiring reaches a session builder.
    pub(crate) fn with_tool_sets(
        mut self,
        chat: Arc<RegisteredToolSet>,
        subagent: Arc<RegisteredToolSet>,
    ) -> Self {
        self.chat_tools = Some(chat);
        self.subagent_tools = Some(subagent);
        self
    }

    /// Returns the terminal boundary used by the configured session hooks.
    pub(crate) fn terminal_io(&self) -> Arc<TerminalIO> {
        self.terminal_io.clone()
    }

    /// Returns the HTTP client used by the provider session and fetch tool composition.
    pub(crate) fn http_client(&self) -> Arc<Client> {
        self.http_client.clone()
    }

    /// Refreshes selected MCP tools before a replacement session is built.
    pub(crate) async fn reselect_mcp(&self) {
        self.mcp_registry.reselect_tools().await;
    }

    /// Returns the goal capability selected by the composition root.
    pub(crate) fn goal_completer(&self) -> anyhow::Result<Arc<dyn GoalCompleter>> {
        self.goal_completer
            .clone()
            .ok_or_else(|| anyhow::anyhow!("goal completer was not configured for the chat agent"))
    }

    pub(super) fn tool_set(&self, subagent: bool) -> anyhow::Result<Arc<RegisteredToolSet>> {
        let tools = if subagent {
            &self.subagent_tools
        } else {
            &self.chat_tools
        };
        tools
            .clone()
            .ok_or_else(|| anyhow::anyhow!("agent tools were not configured"))
    }

    pub(super) fn model_selector(&self) -> anyhow::Result<Arc<dyn ModelSelector>> {
        self.model_selector
            .clone()
            .ok_or_else(|| anyhow::anyhow!("model selector was not configured"))
    }

    pub(crate) fn prompt_provider(&self) -> anyhow::Result<Arc<dyn PromptProvider>> {
        self.prompt_provider
            .clone()
            .ok_or_else(|| anyhow::anyhow!("prompt provider was not configured"))
    }

    /// Returns the configured recovery workflow or reports missing composition.
    pub(crate) fn turn_recovery(&self) -> anyhow::Result<Arc<dyn TurnRecovery>> {
        self.turn_recovery
            .clone()
            .ok_or_else(|| anyhow::anyhow!("turn recovery workflow was not configured"))
    }

    pub(super) fn recovery_policy_factory(
        &self,
    ) -> anyhow::Result<Arc<dyn ToolRecoveryPolicyFactory>> {
        self.recovery_policy_factory
            .clone()
            .ok_or_else(|| anyhow::anyhow!("tool recovery policy factory was not configured"))
    }

    pub(super) fn permission_manager(&self) -> anyhow::Result<Arc<dyn ToolPermissionEvaluator>> {
        self.permission_manager
            .as_ref()
            .map(|factory| factory())
            .ok_or_else(|| anyhow::anyhow!("permission manager was not configured"))
    }
}
