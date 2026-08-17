use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use rig::{
    agent::{AgentHook, HookContext, ToolCall, ToolCallAction},
    tool::Tool,
};

use crate::{entities::tool_confirm_result::ToolConfirmResult, shared::terminal::TerminalIO};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PermissionRequirement {
    Automatic,
    ConfirmationRequired,
}

pub(crate) trait ToolPermissionMetadata {
    fn permission_requirement(&self) -> PermissionRequirement;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ToolOrigin {
    BuiltIn,
    Mcp,
}

#[derive(Clone, Copy)]
struct PermissionEntry {
    requirement: PermissionRequirement,
    origin: ToolOrigin,
}

#[derive(Default)]
pub(crate) struct ToolPermissionCatalog {
    entries: HashMap<String, PermissionEntry>,
}

impl ToolPermissionCatalog {
    pub(crate) fn register_tool<T>(&mut self, tool: &T)
    where
        T: Tool + ToolPermissionMetadata,
    {
        let name = T::NAME.to_string();
        let mcp_entry = self
            .entries
            .get(&name)
            .is_some_and(|entry| entry.origin == ToolOrigin::Mcp);
        if mcp_entry {
            return;
        }
        self.entries.insert(
            name,
            PermissionEntry {
                requirement: tool.permission_requirement(),
                origin: ToolOrigin::BuiltIn,
            },
        );
    }

    pub(crate) fn register_mcp_tool(&mut self, name: &str) {
        self.entries.insert(
            name.to_string(),
            PermissionEntry {
                requirement: PermissionRequirement::ConfirmationRequired,
                origin: ToolOrigin::Mcp,
            },
        );
    }

    fn requirement(&self, name: &str) -> PermissionRequirement {
        self.entries
            .get(name)
            .map_or(PermissionRequirement::ConfirmationRequired, |entry| {
                entry.requirement
            })
    }

    pub(crate) fn register_builtin_tool<T>(&mut self, tool: T) -> T
    where
        T: Tool + ToolPermissionMetadata,
    {
        self.register_tool(&tool);
        tool
    }
}

pub(crate) struct ToolPermissionRequest<'a> {
    tool_name: &'a str,
}

impl<'a> ToolPermissionRequest<'a> {
    fn new(tool_name: &'a str) -> Self {
        Self { tool_name }
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
    prompt_lock: Mutex<()>,
}

impl ToolPermissionHook {
    pub(crate) fn new(
        prompter: Arc<dyn ToolPermissionPrompter>,
        catalog: ToolPermissionCatalog,
    ) -> Self {
        Self {
            prompter,
            catalog,
            prompt_lock: Mutex::new(()),
        }
    }

    fn decide(&self, request: &ToolPermissionRequest<'_>) -> ToolCallAction {
        if self.catalog.requirement(request.tool_name) == PermissionRequirement::Automatic {
            return ToolCallAction::run();
        }
        match self.confirm(request) {
            ToolConfirmResult::AllowOnce => ToolCallAction::run(),
            ToolConfirmResult::No => ToolCallAction::skip(denial_message(request)),
        }
    }

    fn confirm(&self, request: &ToolPermissionRequest<'_>) -> ToolConfirmResult {
        match self.prompt_lock.lock() {
            Ok(_guard) => self.prompter.confirm(request),
            Err(_) => ToolConfirmResult::No,
        }
    }
}

impl AgentHook for ToolPermissionHook {
    async fn on_tool_call(&self, _ctx: &HookContext, event: ToolCall<'_>) -> ToolCallAction {
        let request = ToolPermissionRequest::new(event.tool_name);
        self.decide(&request)
    }
}

fn denial_message(request: &ToolPermissionRequest<'_>) -> String {
    format!(
        "The user did not approve tool call `{}`; do not retry it without new approval.",
        request.tool_name,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use rig::agent::ToolCallAction;

    use super::{
        PermissionRequirement, ToolPermissionCatalog, ToolPermissionHook, ToolPermissionPrompter,
        ToolPermissionRequest,
    };
    use crate::entities::tool_confirm_result::ToolConfirmResult;

    struct StubPrompter {
        decision: ToolConfirmResult,
        requests: Mutex<Vec<String>>,
    }

    impl StubPrompter {
        fn new(decision: ToolConfirmResult) -> Self {
            Self {
                decision,
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl ToolPermissionPrompter for StubPrompter {
        fn confirm(&self, request: &ToolPermissionRequest<'_>) -> ToolConfirmResult {
            if let Ok(mut requests) = self.requests.lock() {
                requests.push(request.prompt());
            }
            self.decision
        }
    }

    #[test]
    fn unknown_tools_require_confirmation_by_default() {
        let catalog = ToolPermissionCatalog::default();
        assert_eq!(
            catalog.requirement("unregistered_tool"),
            PermissionRequirement::ConfirmationRequired
        );
    }

    #[test]
    fn mcp_tools_require_confirmation_even_when_names_overlap() {
        let mut catalog = ToolPermissionCatalog::default();
        catalog.entries.insert(
            "shared_runtime_name".to_string(),
            super::PermissionEntry {
                requirement: PermissionRequirement::Automatic,
                origin: super::ToolOrigin::BuiltIn,
            },
        );
        catalog.register_mcp_tool("shared_runtime_name");
        let prompter = Arc::new(StubPrompter::new(ToolConfirmResult::No));
        let hook = ToolPermissionHook::new(prompter, catalog);
        let request = ToolPermissionRequest::new("shared_runtime_name");

        assert!(matches!(hook.decide(&request), ToolCallAction::Skip(_)));
    }

    #[test]
    fn denial_is_a_skipped_permission_result() {
        let prompter = Arc::new(StubPrompter::new(ToolConfirmResult::No));
        let hook = ToolPermissionHook::new(prompter, ToolPermissionCatalog::default());
        let request = ToolPermissionRequest::new("dangerous_tool");

        let action = hook.decide(&request);

        assert!(matches!(&action, ToolCallAction::Skip(_)));
        assert!(
            matches!(action, ToolCallAction::Skip(reason) if reason.contains("did not approve"))
        );
    }

    #[test]
    fn allow_once_runs_the_tool() {
        let prompter = Arc::new(StubPrompter::new(ToolConfirmResult::AllowOnce));
        let hook = ToolPermissionHook::new(prompter, ToolPermissionCatalog::default());
        let request = ToolPermissionRequest::new("dangerous_tool");

        assert_eq!(hook.decide(&request), ToolCallAction::Run);
    }

    #[test]
    fn automatic_tools_do_not_prompt() {
        let prompter = Arc::new(StubPrompter::new(ToolConfirmResult::No));
        let mut catalog = ToolPermissionCatalog::default();
        catalog.entries.insert(
            "safe_tool".to_string(),
            super::PermissionEntry {
                requirement: PermissionRequirement::Automatic,
                origin: super::ToolOrigin::BuiltIn,
            },
        );
        let hook = ToolPermissionHook::new(prompter.clone(), catalog);
        let request = ToolPermissionRequest::new("safe_tool");

        assert_eq!(hook.decide(&request), ToolCallAction::Run);
        assert!(
            prompter
                .requests
                .lock()
                .is_ok_and(|requests| requests.is_empty())
        );
    }

    #[test]
    fn prompt_uses_only_runtime_tool_identity() {
        let request = ToolPermissionRequest::new("external_tool");

        assert_eq!(request.prompt(), "Allow tool call `external_tool`?");
    }
}
