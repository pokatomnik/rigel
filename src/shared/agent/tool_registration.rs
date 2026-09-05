use rig::{
    agent::{AgentBuilder, WithBuilderTools},
    completion::CompletionModel as CompletionModelTrait,
};

use super::{agent::Agent, tool_build_context::ToolBuildContext};
use crate::{
    shared::{
        goal::goal_completion_hook::GoalCompletionHook,
        tool_permissions::catalog::ToolPermissionCatalog,
    },
    tools::{
        tool_fetch_url::FetchUrl, tool_mark_goal_complete::MarkGoalComplete,
        tool_read_file::ReadFile, tool_run_command::RunCommand, tool_spawn_subagent::SpawnSubagent,
    },
};

impl Agent {
    pub(super) async fn add_chat_tools<M>(
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        context: ToolBuildContext,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        let goal_state = context.dependencies.goal_state()?;
        let builder = builder
            .tool(catalog.register_builtin_tool(RunCommand::new().await?))
            .tool(catalog.register_builtin_tool(ReadFile::new().await?))
            .tool(
                catalog
                    .register_builtin_tool(FetchUrl::new(context.dependencies.http_client.clone())),
            )
            .tool(
                catalog.register_builtin_tool(
                    SpawnSubagent::new(
                        context.dependencies.terminal_io.clone(),
                        context.dependencies.clone(),
                        context.config,
                    )
                    .await?,
                ),
            )
            .tool(catalog.register_builtin_tool(MarkGoalComplete::new(goal_state.clone())))
            .add_hook(GoalCompletionHook::new(goal_state));
        Ok(builder)
    }

    pub(super) async fn add_subagent_tools<M>(
        builder: AgentBuilder<M>,
        catalog: &mut ToolPermissionCatalog,
        context: ToolBuildContext,
    ) -> anyhow::Result<AgentBuilder<M, WithBuilderTools>>
    where
        M: CompletionModelTrait,
    {
        let builder = builder
            .tool(catalog.register_builtin_tool(RunCommand::new().await?))
            .tool(catalog.register_builtin_tool(ReadFile::new().await?))
            .tool(
                catalog
                    .register_builtin_tool(FetchUrl::new(context.dependencies.http_client.clone())),
            );
        Ok(builder)
    }
}
