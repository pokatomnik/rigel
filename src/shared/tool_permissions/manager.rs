use std::{collections::HashMap, path::Path};

use rig::agent::ToolCallAction;
use tokio::sync::Mutex;

use crate::{
    entities::tool_confirm_result::ToolConfirmResult,
    shared::{
        config::rigel_config::{RigelConfig, ToolPermissionPaths},
        tool_permissions::catalog::PermissionRequirement,
    },
};

use super::hook::{ToolPermissionPrompter, ToolPermissionRequest};

pub(crate) struct ToolPermissionManager {
    session_policies: Mutex<HashMap<String, bool>>,
    decision_lock: Mutex<()>,
    paths: ToolPermissionPaths,
}

impl ToolPermissionManager {
    pub(crate) fn new(paths: ToolPermissionPaths) -> Self {
        Self {
            session_policies: Mutex::new(HashMap::new()),
            decision_lock: Mutex::new(()),
            paths,
        }
    }

    pub(crate) async fn decide(
        &self,
        request: &ToolPermissionRequest<'_>,
        requirement: PermissionRequirement,
        prompter: &dyn ToolPermissionPrompter,
    ) -> ToolCallAction {
        let _decision_guard = self.decision_lock.lock().await;
        if let Some(action) = self.session_action(request).await {
            return action;
        }
        let policy = RigelConfig::policy_from_paths(
            self.paths.global_config_path.as_path(),
            self.paths.project_config_path.as_path(),
            request.tool_name(),
        )
        .await;
        if let Some(action) = self.policy_action(request, policy) {
            return action;
        }
        self.decide_without_policy(request, requirement, prompter)
            .await
    }

    async fn session_action(&self, request: &ToolPermissionRequest<'_>) -> Option<ToolCallAction> {
        self.session_policies
            .lock()
            .await
            .get(request.tool_name())
            .copied()
            .map(|allow| self.action_for_policy(request, allow))
    }

    fn policy_action(
        &self,
        request: &ToolPermissionRequest<'_>,
        policy: Option<bool>,
    ) -> Option<ToolCallAction> {
        policy.map(|allow| self.action_for_policy(request, allow))
    }

    fn action_for_policy(
        &self,
        request: &ToolPermissionRequest<'_>,
        allow: bool,
    ) -> ToolCallAction {
        if allow {
            ToolCallAction::run()
        } else {
            ToolCallAction::skip(Self::policy_denial_message(request))
        }
    }

    async fn decide_without_policy(
        &self,
        request: &ToolPermissionRequest<'_>,
        requirement: PermissionRequirement,
        prompter: &dyn ToolPermissionPrompter,
    ) -> ToolCallAction {
        if requirement == PermissionRequirement::Automatic {
            return ToolCallAction::run();
        }
        self.apply_confirmation(request, prompter.confirm(request))
            .await
    }

    async fn apply_confirmation(
        &self,
        request: &ToolPermissionRequest<'_>,
        result: ToolConfirmResult,
    ) -> ToolCallAction {
        match result {
            ToolConfirmResult::Deny => ToolCallAction::skip(Self::denial_message(request)),
            ToolConfirmResult::AllowOnce => ToolCallAction::run(),
            ToolConfirmResult::AllowForSession => {
                self.session_policies
                    .lock()
                    .await
                    .insert(request.tool_name().to_owned(), true);
                ToolCallAction::run()
            }
            ToolConfirmResult::AllowForProject => {
                self.persist_allowance(self.paths.project_config_path.as_path(), request)
                    .await
            }
            ToolConfirmResult::AllowForever => {
                self.persist_allowance(self.paths.global_config_path.as_path(), request)
                    .await
            }
        }
    }

    async fn persist_allowance(
        &self,
        path: &Path,
        request: &ToolPermissionRequest<'_>,
    ) -> ToolCallAction {
        let _ = RigelConfig::update_policy_file(path, request.tool_name(), true).await;
        ToolCallAction::run()
    }

    fn denial_message(request: &ToolPermissionRequest<'_>) -> String {
        format!(
            "The user did not approve tool call `{}`; do not retry it without new approval.",
            request.tool_name(),
        )
    }

    fn policy_denial_message(request: &ToolPermissionRequest<'_>) -> String {
        format!(
            "Tool call `{}` is denied by the configured permission policy; do not retry it.",
            request.tool_name(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use rig::agent::ToolCallAction;

    use super::ToolPermissionManager;
    use crate::{
        entities::tool_confirm_result::ToolConfirmResult,
        shared::config::rigel_config::ToolPermissionPaths,
        shared::tool_permissions::{
            catalog::PermissionRequirement,
            hook::{ToolPermissionPrompter, ToolPermissionRequest},
        },
    };

    struct StubPrompter {
        results: Mutex<Vec<ToolConfirmResult>>,
        prompts: Mutex<usize>,
    }

    fn manager() -> ToolPermissionManager {
        ToolPermissionManager::new(ToolPermissionPaths {
            global_config_path: "global.toml".into(),
            project_config_path: "project.toml".into(),
        })
    }

    impl StubPrompter {
        fn new(results: Vec<ToolConfirmResult>) -> Self {
            Self {
                results: Mutex::new(results),
                prompts: Mutex::new(0),
            }
        }

        fn prompt_count(&self) -> Option<usize> {
            self.prompts.lock().ok().map(|count| *count)
        }
    }

    impl ToolPermissionPrompter for StubPrompter {
        fn confirm(&self, _request: &ToolPermissionRequest<'_>) -> ToolConfirmResult {
            if let Ok(mut prompts) = self.prompts.lock() {
                *prompts += 1;
            }
            self.results
                .lock()
                .ok()
                .and_then(|mut results| results.pop())
                .unwrap_or_default()
        }
    }

    #[tokio::test]
    async fn session_allowance_is_reused_without_prompting_again() {
        let manager = manager();
        let prompter = StubPrompter::new(vec![
            ToolConfirmResult::Deny,
            ToolConfirmResult::AllowForSession,
        ]);
        let request = ToolPermissionRequest::new("run_command");

        let first = manager
            .apply_confirmation(&request, prompter.confirm(&request))
            .await;
        let second = manager.session_action(&request).await;

        assert_eq!(first, ToolCallAction::Run);
        assert_eq!(second, Some(ToolCallAction::Run));
        assert_eq!(prompter.prompt_count(), Some(1));
    }

    #[tokio::test]
    async fn explicit_deny_skips_even_automatic_tools_without_prompting() {
        let manager = manager();
        let prompter = StubPrompter::new(vec![ToolConfirmResult::Deny]);
        let request = ToolPermissionRequest::new("fetch_url");

        let action = manager.action_for_policy(&request, false);

        assert!(matches!(action, ToolCallAction::Skip(reason) if reason.contains("policy")));
        assert_eq!(prompter.prompt_count(), Some(0));
    }

    #[tokio::test]
    async fn automatic_tools_run_without_policy_or_prompt() {
        let manager = manager();
        let prompter = StubPrompter::new(vec![ToolConfirmResult::Deny]);
        let request = ToolPermissionRequest::new("fetch_url");

        let action = manager
            .decide_without_policy(&request, PermissionRequirement::Automatic, &prompter)
            .await;

        assert_eq!(action, ToolCallAction::Run);
        assert_eq!(prompter.prompt_count(), Some(0));
    }

    #[tokio::test]
    async fn allow_once_does_not_survive_the_next_call() {
        let manager = manager();
        let prompter =
            StubPrompter::new(vec![ToolConfirmResult::Deny, ToolConfirmResult::AllowOnce]);
        let request = ToolPermissionRequest::new("run_command");

        let first = manager
            .decide_without_policy(
                &request,
                PermissionRequirement::ConfirmationRequired,
                &prompter,
            )
            .await;
        let second = manager
            .decide_without_policy(
                &request,
                PermissionRequirement::ConfirmationRequired,
                &prompter,
            )
            .await;

        assert_eq!(first, ToolCallAction::Run);
        assert!(matches!(second, ToolCallAction::Skip(_)));
        assert_eq!(prompter.prompt_count(), Some(2));
    }
}
