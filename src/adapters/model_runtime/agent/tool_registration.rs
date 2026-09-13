use std::sync::Arc;

use rig::{
    agent::{AgentBuilder, WithBuilderTools},
    completion::CompletionModel as CompletionModelTrait,
};

use super::{agent::Agent, tool_build_context::ToolBuildContext};
use crate::{
    adapters::model_runtime::permissions::catalog::ToolPermissionCatalog,
    adapters::{
        model_runtime::{
            goal_completion_hook::GoalCompletionHook, http_fetcher::ReqwestSafeHttpFetcher,
            subagent::RuntimeSubagentRunner,
        },
        terminal::{
            command_executor::{GlobalShellUsageRecorder, ProcessCommandExecutor},
            user_interaction::TerminalInteraction,
        },
    },
    tools::{
        tool_ask_user::ask_user::AskUser, tool_edit_file::edit_file::EditFile,
        tool_fetch_url::fetch_url::FetchUrl, tool_glob_files::glob_files::GlobFiles,
        tool_mark_goal_complete::mark_goal_complete::MarkGoalComplete,
        tool_read_file::read_file::ReadFile, tool_run_command::run_command::RunCommand,
        tool_search_files::search_files::SearchFiles,
        tool_spawn_subagent::spawn_subagent::SpawnSubagent, tool_write_file::write_file::WriteFile,
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
        let goal_completer = context.dependencies.goal_completer()?;
        let workspace = context.dependencies.workspace()?;
        let ask_user = AskUser::with_interaction(Arc::new(TerminalInteraction::new(
            context.dependencies.terminal_io.clone(),
        )));

        let run_command = RunCommand::with_adapters(
            workspace.root(),
            Arc::new(ProcessCommandExecutor::new_default()),
            Arc::new(GlobalShellUsageRecorder::new()),
        );
        let read_file =
            ReadFile::new(workspace.path_resolver.clone(), workspace.reader.clone()).await?;
        let edit_file = EditFile::new(
            workspace.path_resolver.clone(),
            workspace.reader.clone(),
            workspace.writer.clone(),
        )
        .await?;
        let write_file =
            WriteFile::new(workspace.path_resolver.clone(), workspace.writer.clone()).await?;
        let search_files = SearchFiles::new(
            workspace.path_resolver.clone(),
            workspace.reader.clone(),
            workspace.scanner.clone(),
        )
        .await?;
        let glob_files =
            GlobFiles::new(workspace.path_resolver.clone(), workspace.scanner.clone()).await?;
        let fetch_url = FetchUrl::with_fetcher(Arc::new(ReqwestSafeHttpFetcher::new(
            context.dependencies.http_client.clone(),
        )));
        let runner = RuntimeSubagentRunner::build(
            context.dependencies.terminal_io.clone(),
            context.dependencies.clone(),
            context.config,
        )
        .await?;
        let spawn_subagent = SpawnSubagent::with_runner(runner);
        let mark_goal_complete = MarkGoalComplete::new(goal_completer.clone());

        let goal_complete_hook = GoalCompletionHook::new(goal_completer);

        let builder = builder
            .tool(catalog.register_builtin_tool(ask_user))
            .tool(catalog.register_builtin_tool(run_command))
            .tool(catalog.register_builtin_tool(read_file))
            .tool(catalog.register_builtin_tool(edit_file))
            .tool(catalog.register_builtin_tool(write_file))
            .tool(catalog.register_builtin_tool(search_files))
            .tool(catalog.register_builtin_tool(glob_files))
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
        let workspace = context.dependencies.workspace()?;
        let run_command = RunCommand::with_adapters(
            workspace.root(),
            Arc::new(ProcessCommandExecutor::new_default()),
            Arc::new(GlobalShellUsageRecorder::new()),
        );
        let read_file =
            ReadFile::new(workspace.path_resolver.clone(), workspace.reader.clone()).await?;
        let edit_file = EditFile::new(
            workspace.path_resolver.clone(),
            workspace.reader.clone(),
            workspace.writer.clone(),
        )
        .await?;
        let write_file =
            WriteFile::new(workspace.path_resolver.clone(), workspace.writer.clone()).await?;
        let search_files = SearchFiles::new(
            workspace.path_resolver.clone(),
            workspace.reader.clone(),
            workspace.scanner.clone(),
        )
        .await?;
        let glob_files =
            GlobFiles::new(workspace.path_resolver.clone(), workspace.scanner.clone()).await?;
        let search_url = FetchUrl::with_fetcher(Arc::new(ReqwestSafeHttpFetcher::new(
            context.dependencies.http_client.clone(),
        )));

        let builder = builder
            .tool(catalog.register_builtin_tool(run_command))
            .tool(catalog.register_builtin_tool(read_file))
            .tool(catalog.register_builtin_tool(edit_file))
            .tool(catalog.register_builtin_tool(write_file))
            .tool(catalog.register_builtin_tool(search_files))
            .tool(catalog.register_builtin_tool(glob_files))
            .tool(catalog.register_builtin_tool(search_url));
        Ok(builder)
    }
}
