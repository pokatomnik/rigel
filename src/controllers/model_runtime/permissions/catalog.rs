use std::collections::HashMap;

use crate::entities::tool::tool_entities::PermissionRequirement;

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
    /// Registers a type-erased built-in tool without overriding an MCP name collision.
    pub(crate) fn register_dynamic_tool(&mut self, name: &str, requirement: PermissionRequirement) {
        self.register(name, requirement, ToolOrigin::BuiltIn);
    }

    /// Registers an MCP tool, which takes precedence over colliding built-ins.
    pub(crate) fn register_mcp_tool(&mut self, name: &str) {
        self.register(
            name,
            PermissionRequirement::ConfirmationRequired,
            ToolOrigin::Mcp,
        );
    }

    fn register(&mut self, name: &str, requirement: PermissionRequirement, origin: ToolOrigin) {
        let mcp_has_precedence = self
            .entries
            .get(name)
            .is_some_and(|entry| entry.origin == ToolOrigin::Mcp)
            && origin == ToolOrigin::BuiltIn;
        if mcp_has_precedence {
            return;
        }
        self.entries.insert(
            name.to_string(),
            PermissionEntry {
                requirement,
                origin,
            },
        );
    }

    /// Returns confirmation-required for names absent from the current session catalog.
    pub(crate) fn requirement(&self, name: &str) -> PermissionRequirement {
        self.entries
            .get(name)
            .map_or(PermissionRequirement::ConfirmationRequired, |entry| {
                entry.requirement
            })
    }
}

#[cfg(test)]
mod tests {
    use super::ToolPermissionCatalog;
    use crate::entities::tool::tool_entities::PermissionRequirement;

    #[test]
    fn unknown_tools_require_confirmation_by_default() {
        let catalog = ToolPermissionCatalog::default();

        assert_eq!(
            catalog.requirement("unregistered_tool"),
            PermissionRequirement::ConfirmationRequired
        );
    }

    #[test]
    fn mcp_registration_wins_when_builtin_was_registered_first() {
        let mut catalog = ToolPermissionCatalog::default();
        catalog.register_dynamic_tool("shared_runtime_name", PermissionRequirement::Automatic);

        catalog.register_mcp_tool("shared_runtime_name");

        assert_eq!(
            catalog.requirement("shared_runtime_name"),
            PermissionRequirement::ConfirmationRequired
        );
    }

    #[test]
    fn builtin_registration_does_not_override_mcp_when_registered_first() {
        let mut catalog = ToolPermissionCatalog::default();
        catalog.register_mcp_tool("shared_runtime_name");

        catalog.register_dynamic_tool("shared_runtime_name", PermissionRequirement::Automatic);

        assert_eq!(
            catalog.requirement("shared_runtime_name"),
            PermissionRequirement::ConfirmationRequired
        );
    }
}
