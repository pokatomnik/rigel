use crate::entities::tool::tool_entities::PermissionRequirement;

/// Supplies the permission category attached to a registered model tool.
pub(crate) trait ToolPermissionMetadata {
    /// Returns automatic or confirmation-required behavior.
    fn permission_requirement(&self) -> PermissionRequirement;
}
