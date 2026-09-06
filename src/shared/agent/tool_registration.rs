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
        tool_fetch_url::fetch_url::FetchUrl,
        tool_mark_goal_complete::mark_goal_complete::MarkGoalComplete,
        tool_read_file::read_file::ReadFile, tool_run_command::run_command::RunCommand,
        tool_search_files::search_files::SearchFiles,
        tool_spawn_subagent::spawn_subagent::SpawnSubagent,
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

        let run_command = RunCommand::new().await?;
        let read_file = ReadFile::new().await?;
        let search_files = SearchFiles::new().await?;
        let fetch_url = FetchUrl::new(context.dependencies.http_client.clone());
        let spawn_subagent = SpawnSubagent::new(
            context.dependencies.terminal_io.clone(),
            context.dependencies.clone(),
            context.config,
        )
        .await?;
        let mark_goal_complete = MarkGoalComplete::new(goal_state.clone());

        let goal_complete_hook = GoalCompletionHook::new(goal_state);

        let builder = builder
            .tool(catalog.register_builtin_tool(run_command))
            .tool(catalog.register_builtin_tool(read_file))
            .tool(catalog.register_builtin_tool(search_files))
            .tool(catalog.register_builtin_tool(fetch_url))
            .tool(catalog.register_builtin_tool(spawn_subagent))
            .tool(catalog.register_builtin_tool(mark_goal_complete))
            .add_hook(goal_complete_hook);
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
        let run_command = RunCommand::new().await?;
        let read_file = ReadFile::new().await?;
        let search_files = SearchFiles::new().await?;
        let search_url = FetchUrl::new(context.dependencies.http_client.clone());

        let builder = builder
            .tool(catalog.register_builtin_tool(run_command))
            .tool(catalog.register_builtin_tool(read_file))
            .tool(catalog.register_builtin_tool(search_files))
            .tool(catalog.register_builtin_tool(search_url));
        Ok(builder)
    }
}
