use std::collections::HashMap;

use rig::tool::Tool;

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

    pub(crate) fn requirement(&self, name: &str) -> PermissionRequirement {
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

#[cfg(test)]
mod tests {
    use super::{PermissionRequirement, ToolPermissionCatalog};

    #[test]
    fn unknown_tools_require_confirmation_by_default() {
        let catalog = ToolPermissionCatalog::default();

        assert_eq!(
            catalog.requirement("unregistered_tool"),
            PermissionRequirement::ConfirmationRequired
        );
    }

    #[test]
    fn mcp_registration_wins_over_a_builtin_with_the_same_name() {
        let mut catalog = ToolPermissionCatalog::default();
        catalog.entries.insert(
            "shared_runtime_name".to_string(),
            super::PermissionEntry {
                requirement: PermissionRequirement::Automatic,
                origin: super::ToolOrigin::BuiltIn,
            },
        );

        catalog.register_mcp_tool("shared_runtime_name");

        assert_eq!(
            catalog.requirement("shared_runtime_name"),
            PermissionRequirement::ConfirmationRequired
        );
    }
}
