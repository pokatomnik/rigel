use std::sync::Arc;

use rig::agent::{AgentHook, HookContext, ToolCall, ToolCallAction};

use crate::{
    entities::tool_confirm_result::ToolConfirmResult, shared::terminal::terminal_io::TerminalIO,
};

use super::{catalog::ToolPermissionCatalog, manager::ToolPermissionManager};

pub(crate) struct ToolPermissionRequest<'a> {
    tool_name: &'a str,
}

impl<'a> ToolPermissionRequest<'a> {
    pub(crate) fn new(tool_name: &'a str) -> Self {
        Self { tool_name }
    }

    pub(crate) fn tool_name(&self) -> &str {
        self.tool_name
    }

    fn prompt(&self) -> String {
        format!("Allow tool call `{}`?", self.tool_name)
    }
}

pub(crate) trait ToolPermissionPrompter: Send + Sync {
    fn confirm(&self, request: &ToolPermissionRequest<'_>) -> ToolConfirmResult;
}

impl ToolPermissionPrompter for TerminalIO {
    fn confirm(&self, request: &ToolPermissionRequest<'_>) -> ToolConfirmResult {
        self.confirm_tool_call(request.prompt().as_str())
    }
}

pub(crate) struct ToolPermissionHook {
    prompter: Arc<dyn ToolPermissionPrompter>,
    catalog: ToolPermissionCatalog,
    manager: ToolPermissionManager,
}

impl ToolPermissionHook {
    pub(crate) fn new(
        prompter: Arc<dyn ToolPermissionPrompter>,
        catalog: ToolPermissionCatalog,
        manager: ToolPermissionManager,
    ) -> Self {
        Self {
            prompter,
            catalog,
            manager,
        }
    }

    async fn decide(&self, request: &ToolPermissionRequest<'_>) -> ToolCallAction {
        let requirement = self.catalog.requirement(request.tool_name());
        self.manager
            .decide(request, requirement, self.prompter.as_ref())
            .await
    }
}

impl AgentHook for ToolPermissionHook {
    async fn on_tool_call(&self, _ctx: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        let request = ToolPermissionRequest::new(event.tool_name);
        self.decide(&request).await
    }
}
