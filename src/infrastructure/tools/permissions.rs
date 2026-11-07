use std::{future::Future, pin::Pin};

use crate::{
    entities::tool::tool_entities::{
        PermissionRequirement, ToolPermissionDecision, ToolPermissionRequest,
    },
    infrastructure::interaction::ToolPermissionPrompter,
};

/// Evaluates one tool call through session, persisted, and interactive policy.
pub(crate) trait ToolPermissionEvaluator: Send + Sync {
    /// Returns a domain decision; provider-specific hooks map it to their action type.
    fn decide<'a>(
        &'a self,
        request: &'a ToolPermissionRequest,
        requirement: PermissionRequirement,
        prompter: &'a dyn ToolPermissionPrompter,
    ) -> Pin<Box<dyn Future<Output = ToolPermissionDecision> + Send + 'a>>;
}

/// Supplies the permission category attached to a registered model tool.
pub(crate) trait ToolPermissionMetadata {
    /// Returns automatic or confirmation-required behavior.
    fn permission_requirement(&self) -> PermissionRequirement;
}
