use std::sync::Arc;

use rig::{
    agent::{AgentBuilder, WithBuilderTools},
    completion::CompletionModel,
    tool::{DynamicTool, Tool},
};

use crate::{
    controllers::model_runtime::permissions::catalog::ToolPermissionCatalog,
    entities::tool::tool_entities::PermissionRequirement,
    infrastructure::tools::permissions::ToolPermissionMetadata,
};

fn erase_tool<T>(tool: T) -> DynamicTool
where
    T: Tool + 'static,
{
    let tool = Arc::new(tool);
    DynamicTool::new(
        T::NAME,
        tool.description(),
        tool.parameters(),
        move |context, arguments| {
            let tool = tool.clone();
            Box::pin(async move {
                let args = serde_json::from_value(arguments).map_err(|error| {
                    rig::tool::ToolExecutionError::invalid_args(format!(
                        "failed to parse tool arguments: {error}"
                    ))
                })?;
                let output = tool
                    .call(context, args)
                    .await
                    .map_err(|error| tool.map_error(error))?;
                rig::tool::IntoToolOutput::into_tool_output(output)
            })
        },
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentToolSet {
    Subagent,
    Chat,
}

/// A composition-root-owned collection of type-erased tools and their policies.
#[derive(Clone, Default)]
pub(crate) struct RegisteredToolSet {
    tools: Vec<RegisteredTool>,
}

#[derive(Clone)]
struct RegisteredTool {
    tool: DynamicTool,
    requirement: PermissionRequirement,
}

impl RegisteredToolSet {
    /// Creates an empty tool set; the composition root adds tools in model-facing order.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds one typed tool while preserving its permission metadata.
    pub(crate) fn add<T>(&mut self, tool: T)
    where
        T: Tool + ToolPermissionMetadata + 'static,
    {
        self.tools.push(RegisteredTool {
            requirement: tool.permission_requirement(),
            tool: erase_tool(tool),
        });
    }

    /// Returns registered names and permission requirements for composition tests.
    #[cfg(test)]
    pub(crate) fn test_metadata(&self) -> Vec<(String, PermissionRequirement)> {
        self.tools
            .iter()
            .map(|registered| (registered.tool.name().to_string(), registered.requirement))
            .collect()
    }

    /// Registers this set in the Rig builder without exposing concrete tool types to the agent.
    pub(crate) fn add_to_builder<M>(
        &self,
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModel,
    {
        let Some(first) = self.tools.first() else {
            anyhow::bail!("agent tool set cannot be empty")
        };
        let mut builder = builder.dynamic_tool(first.tool.clone());
        catalog.register_dynamic_tool(first.tool.name(), first.requirement);
        for registered in self.tools.iter().skip(1) {
            catalog.register_dynamic_tool(registered.tool.name(), registered.requirement);
            builder = builder.dynamic_tool(registered.tool.clone());
        }
        Ok(builder)
    }
}
