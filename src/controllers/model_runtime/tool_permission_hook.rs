use std::sync::Arc;

use rig::agent::{AgentHook, HookContext, ToolCall, ToolCallAction};

use crate::{
    controllers::model_runtime::permissions::catalog::ToolPermissionCatalog,
    controllers::terminal::terminal_io::TerminalIO,
    entities::{
        tool::tool_entities::{ToolPermissionDecision, ToolPermissionRequest},
        tool_confirm_result::outcome::ToolConfirmResult,
    },
    infrastructure::{
        interaction::ToolPermissionPrompter, tools::permissions::ToolPermissionEvaluator,
    },
};

/// Thin Rig adapter that maps tool-call events into the permission workflow.
pub(crate) struct ToolPermissionHook {
    prompter: Arc<dyn ToolPermissionPrompter>,
    catalog: ToolPermissionCatalog,
    manager: Arc<dyn ToolPermissionEvaluator>,
}

impl ToolPermissionHook {
    /// Creates the hook with a session catalog and policy manager.
    pub(crate) fn new(
        prompter: Arc<dyn ToolPermissionPrompter>,
        catalog: ToolPermissionCatalog,
        manager: Arc<dyn ToolPermissionEvaluator>,
    ) -> Self {
        Self {
            prompter,
            catalog,
            manager,
        }
    }

    async fn decide(&self, request: &ToolPermissionRequest) -> ToolCallAction {
        let requirement = self.catalog.requirement(request.tool_name());
        match self
            .manager
            .decide(request, requirement, self.prompter.as_ref())
            .await
        {
            ToolPermissionDecision::Allow => ToolCallAction::run(),
            ToolPermissionDecision::Deny(reason) => ToolCallAction::skip(reason),
        }
    }
}

impl ToolPermissionPrompter for TerminalIO {
    fn confirm(&self, request: &ToolPermissionRequest) -> ToolConfirmResult {
        self.confirm_tool_call(format!("Allow tool call `{}`?", request.tool_name()).as_str())
    }
}

impl AgentHook for ToolPermissionHook {
    async fn on_tool_call(&self, _ctx: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        let request = ToolPermissionRequest::new(event.tool_name.to_string());
        self.decide(&request).await
    }
}
