use std::sync::Arc;

use rig::agent::{AgentHook, HookContext, ToolCall, ToolCallAction};

use crate::{
    controllers::model_runtime::permissions::catalog::ToolPermissionCatalog,
    entities::tool::tool_entities::{ToolPermissionDecision, ToolPermissionRequest},
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

impl AgentHook for ToolPermissionHook {
    async fn on_tool_call(&self, _ctx: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        let arguments = serde_json::from_str(event.args)
            .unwrap_or_else(|_| serde_json::Value::String(event.args.to_string()));
        let request = ToolPermissionRequest::new(event.tool_name.to_string(), arguments);
        self.decide(&request).await
    }
}
