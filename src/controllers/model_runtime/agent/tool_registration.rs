use rig::{
    agent::{AgentBuilder, WithBuilderTools},
    completion::CompletionModel,
};

use super::runtime::Agent;
use crate::controllers::model_runtime::permissions::catalog::ToolPermissionCatalog;

impl Agent {
    pub(super) fn add_chat_tools<M>(
        &self,
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModel,
    {
        self.wiring
            .tool_set(false)?
            .add_to_builder(builder, catalog)
    }

    pub(super) fn add_subagent_tools<M>(
        &self,
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModel,
    {
        self.wiring.tool_set(true)?.add_to_builder(builder, catalog)
    }
}
