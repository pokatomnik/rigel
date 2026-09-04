use rig::{
    agent::{AgentBuilder, WithBuilderTools},
    completion::CompletionModel as CompletionModelTrait,
};

use crate::shared::tool_permissions::catalog::ToolPermissionCatalog;

use super::{agent::Agent, tool_build_context::ToolBuildContext};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentToolSet {
    Subagent,
    Orchestrator,
    Chat,
}

impl AgentToolSet {
    pub(super) async fn add_tools<M>(
        self,
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        context: ToolBuildContext,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        match self {
            Self::Subagent => Agent::add_subagent_tools(builder, catalog, context).await,
            Self::Orchestrator => Agent::add_orchestrator_tools(builder, catalog, context).await,
            Self::Chat => Agent::add_chat_tools(builder, catalog, context).await,
        }
    }
}
