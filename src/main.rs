use std::sync::Arc;

use clap::Parser;

use crate::{
    cmd::{cli::Cli, commands::Commands},
    controllers::{
        chat_controller::{command_parser::CommandParser, controller::ChatController},
        configuration::{
            policy_repository::FileToolPolicyRepository, repository::FileConfigurationRepository,
        },
        init_controller::controller::InitController,
        mcp_registry::registry::McpRegistry,
        model_runtime::{
            agent::{
                agent_config::AgentConfig, agent_tool_set::RegisteredToolSet,
                dependencies::AgentWiring,
            },
            history::{chat_history::ChatHistory, persistence::History},
            http_fetcher::ReqwestSafeHttpFetcher,
            model_selector::selection::TerminalModelSelector,
            model_session::RigModelSessionFactory,
            subagent_session_factory::RigSubagentSessionFactory,
        },
        network::http_client::build_client,
        skills::catalog::SkillCatalog,
        terminal::{
            command_executor::{CommandUsageTelemetry, ProcessCommandExecutor},
            terminal_io::TerminalIO,
            user_interaction::TerminalInteraction,
        },
        workspace::filesystem::WorkspaceCapabilities,
    },
    entities::{
        application::{chat_request::ChatRequest, init_request::InitRequest},
        goal::goal_state::GoalState,
        model::model_session_kind::ModelSessionKind,
    },
    infrastructure::{
        application::ConfigurationInitializer,
        configuration::ConfigurationRepository,
        model::ModelSessionFactory,
        tools::{network::SafeHttpFetcher, permissions::ToolPermissionEvaluator},
    },
    prompts::provider::StaticPromptProvider,
    tools::{
        tool_ask_user::ask_user::AskUser,
        tool_edit_file::edit_file::EditFile,
        tool_fetch_url::{fetch_url::FetchUrl, policy::is_allowed_network_target},
        tool_glob_files::glob_files::GlobFiles,
        tool_mark_goal_complete::mark_goal_complete::MarkGoalComplete,
        tool_read_file::read_file::ReadFile,
        tool_run_command::run_command::RunCommand,
        tool_search_files::search_files::SearchFiles,
        tool_spawn_subagent::spawn_subagent::SpawnSubagent,
        tool_write_file::write_file::WriteFile,
    },
    use_cases::{
        chat::{
            compaction::compact_context::CompactContext,
            recovery::policy::DefaultToolRecoveryPolicyFactory,
        },
        chat::{
            history::conversation_history::ConversationHistory as DomainConversationHistory,
            recovery::turn_recovery_workflow::TurnRecoveryWorkflow, workflow::Chat,
        },
        goal::goal_completer::GoalStateCompleter,
        init::initialize::InitializeConfiguration,
        permissions::manager::ToolPermissionManager,
        subagent::workflow::RunSubagent,
    },
};

pub mod cmd;
pub mod controllers;
pub mod entities;
pub mod infrastructure;
pub mod prompts;
pub mod shared;
pub mod tools;
pub mod use_cases;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let terminal_io = Arc::new(TerminalIO);
    let result = dispatch(cli.command, terminal_io.clone()).await;

    if let Err(error) = result {
        terminal_io.eprintln(error.to_string().as_str());
    }
    Ok(())
}
async fn dispatch(command: Commands, terminal_io: Arc<TerminalIO>) -> anyhow::Result<()> {
    match command {
        Commands::Init(args) => initialize(args).await,
        Commands::Chat(args) => run_chat(args, terminal_io).await,
    }
}
async fn initialize(args: crate::cmd::commands::InitArgs) -> anyhow::Result<()> {
    let request = map_init_request(args);
    let repository = Arc::new(FileConfigurationRepository::new());
    let initializer: Arc<dyn ConfigurationInitializer> =
        Arc::new(InitializeConfiguration::new(repository));
    InitController::new(initializer).handle(request).await
}

async fn run_chat(
    args: crate::cmd::commands::ChatArgs,
    terminal_io: Arc<TerminalIO>,
) -> anyhow::Result<()> {
    let (request, wiring, agent_config, goal_state) =
        prepare_chat(args, terminal_io.clone()).await?;

    let (history, messages) = History::bootstrap().await?;
    let domain_history = Arc::new(DomainConversationHistory::new(messages, history));
    let provider_history = Arc::new(ChatHistory::<History>::new(domain_history.clone()));
    let session_factory = Arc::new(RigModelSessionFactory::new(
        agent_config.clone(),
        wiring.clone(),
        provider_history,
    ));
    let session = session_factory.create(ModelSessionKind::Chat, None).await?;
    let chat = Chat::from_session(session, domain_history, terminal_io.clone())
        .with_command_parser(Arc::new(CommandParser))
        .with_skill_repository(Arc::new(SkillCatalog::from_environment()))
        .with_prompt_provider(Arc::new(StaticPromptProvider))
        .with_goal_state(goal_state)
        .with_model_session_factory(session_factory)
        .await;
    ChatController::new(Arc::new(chat), terminal_io)
        .handle(request)
        .await
}
async fn prepare_chat(
    args: crate::cmd::commands::ChatArgs,
    terminal_io: Arc<TerminalIO>,
) -> anyhow::Result<(ChatRequest, Arc<AgentWiring>, AgentConfig, Arc<GoalState>)> {
    let request = map_chat_request(args);
    let repository = FileConfigurationRepository::new();
    let profile_path = request.profile_path();
    let permission_paths = repository.paths(profile_path)?;
    let config = Arc::new(repository.load(profile_path).await?);
    let http_client = build_client(is_allowed_network_target).await?;
    let mcp_registry = Arc::new(McpRegistry::from_config(config.clone()).await?);
    let workspace = workspace_capabilities().await?;
    let goal_state = Arc::new(GoalState::new());
    let goal_completer = Arc::new(GoalStateCompleter::new(goal_state.clone()));
    let policy_repository = Arc::new(FileToolPolicyRepository::new());
    let permission_factory = permission_factory(permission_paths, policy_repository);
    let agent_config = AgentConfig::new(config);
    let wiring = AgentWiring::new(terminal_io.clone(), http_client.clone(), mcp_registry)
        .with_goal_completer(goal_completer.clone())
        .with_prompt_provider(Arc::new(StaticPromptProvider))
        .with_model_selector(Arc::new(TerminalModelSelector::new(terminal_io.clone())))
        .with_permission_manager(permission_factory)
        .with_recovery_policy_factory(Arc::new(DefaultToolRecoveryPolicyFactory::new()))
        .with_turn_recovery(Arc::new(TurnRecoveryWorkflow::new(Arc::new(
            StaticPromptProvider,
        ))))
        .with_context_compaction(Arc::new(CompactContext));
    let (chat_tools, subagent_tools) =
        build_tool_sets(&wiring, workspace.as_ref(), &agent_config).await?;
    let wiring = Arc::new(wiring.with_tool_sets(chat_tools, subagent_tools));
    Ok((request, wiring, agent_config, goal_state))
}
async fn build_tool_sets(
    wiring: &AgentWiring,
    workspace: &WorkspaceCapabilities,
    config: &AgentConfig,
) -> anyhow::Result<(Arc<RegisteredToolSet>, Arc<RegisteredToolSet>)> {
    let recorder = Arc::new(CommandUsageTelemetry::new());
    let fetcher =
        ReqwestSafeHttpFetcher::shared(wiring.http_client(), Arc::new(fetch_target_allowed));
    let subagent =
        Arc::new(build_subagent_tools(workspace, fetcher.clone(), recorder.clone()).await?);
    let subagent_wiring = Arc::new(
        wiring
            .clone()
            .with_tool_sets(Arc::new(RegisteredToolSet::new()), subagent.clone()),
    );
    let runner = Arc::new(RunSubagent::new(
        Arc::new(RigSubagentSessionFactory::new(
            config.clone(),
            subagent_wiring,
        )),
        wiring.prompt_provider()?,
        wiring.terminal_io(),
    ));
    let mut chat = RegisteredToolSet::new();
    chat.add(AskUser::with_interaction(Arc::new(
        TerminalInteraction::new(wiring.terminal_io()),
    )));
    add_workspace_tools(&mut chat, workspace, recorder).await?;
    chat.add(FetchUrl::with_fetcher(fetcher));
    chat.add(SpawnSubagent::with_runner(runner));
    chat.add(MarkGoalComplete::new(wiring.goal_completer()?));
    Ok((Arc::new(chat), subagent))
}
fn map_init_request(args: crate::cmd::commands::InitArgs) -> InitRequest {
    InitRequest::new(
        args.base_url().to_string(),
        args.api_key_env().map(ToOwned::to_owned),
    )
}
async fn workspace_capabilities() -> anyhow::Result<Arc<WorkspaceCapabilities>> {
    let current_dir = std::env::current_dir()?;
    let root = tokio::fs::canonicalize(current_dir).await?;
    Ok(Arc::new(WorkspaceCapabilities::new(root)))
}
fn map_chat_request(args: crate::cmd::commands::ChatArgs) -> ChatRequest {
    ChatRequest::new(args.profile_path().map(ToOwned::to_owned))
}
fn permission_factory(
    paths: crate::entities::configuration::paths::ConfigurationPaths,
    repository: Arc<FileToolPolicyRepository>,
) -> Arc<dyn Fn() -> Arc<dyn ToolPermissionEvaluator> + Send + Sync> {
    Arc::new(move || {
        Arc::new(ToolPermissionManager::new(
            paths.clone(),
            repository.clone(),
        )) as Arc<dyn ToolPermissionEvaluator>
    })
}

async fn build_subagent_tools(
    workspace: &WorkspaceCapabilities,
    fetcher: Arc<dyn SafeHttpFetcher>,
    recorder: Arc<CommandUsageTelemetry>,
) -> anyhow::Result<RegisteredToolSet> {
    let mut tools = RegisteredToolSet::new();
    add_workspace_tools(&mut tools, workspace, recorder).await?;
    tools.add(FetchUrl::with_fetcher(fetcher));
    Ok(tools)
}

async fn add_workspace_tools(
    tools: &mut RegisteredToolSet,
    workspace: &WorkspaceCapabilities,
    recorder: Arc<CommandUsageTelemetry>,
) -> anyhow::Result<()> {
    tools.add(RunCommand::with_adapters(
        workspace.root(),
        Arc::new(ProcessCommandExecutor::new_default()),
        recorder,
    ));
    add_file_tools(tools, workspace).await?;
    add_search_tools(tools, workspace).await
}

async fn add_file_tools(
    tools: &mut RegisteredToolSet,
    workspace: &WorkspaceCapabilities,
) -> anyhow::Result<()> {
    tools.add(ReadFile::new(workspace.path_resolver.clone(), workspace.reader.clone()).await?);
    tools.add(
        EditFile::new(
            workspace.path_resolver.clone(),
            workspace.reader.clone(),
            workspace.writer.clone(),
        )
        .await?,
    );
    tools.add(WriteFile::new(workspace.path_resolver.clone(), workspace.writer.clone()).await?);
    Ok(())
}

async fn add_search_tools(
    tools: &mut RegisteredToolSet,
    workspace: &WorkspaceCapabilities,
) -> anyhow::Result<()> {
    tools.add(
        SearchFiles::new(
            workspace.path_resolver.clone(),
            workspace.reader.clone(),
            workspace.scanner.clone(),
        )
        .await?,
    );
    tools.add(GlobFiles::new(workspace.path_resolver.clone(), workspace.scanner.clone()).await?);
    Ok(())
}

fn fetch_target_allowed(target: &str) -> bool {
    reqwest::Url::parse(target).is_ok_and(|url| is_allowed_network_target(&url))
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        path::{Path, PathBuf},
        pin::Pin,
        sync::Arc,
    };

    use anyhow::{Result, ensure};
    use clap::Parser;

    use super::{add_workspace_tools, build_subagent_tools, map_chat_request, map_init_request};
    type FakeWorkspaceFuture<'a, T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

    struct FakeWorkspace;

    impl WorkspacePathResolver for FakeWorkspace {
        fn root(&self) -> PathBuf {
            PathBuf::from("/workspace")
        }
        fn resolve(&self, path: &str) -> anyhow::Result<PathBuf> {
            Ok(PathBuf::from("/workspace").join(path))
        }
        fn canonicalize(&self, path: &Path) -> FakeWorkspaceFuture<'_, PathBuf> {
            let path = path.to_path_buf();
            Box::pin(async move { Ok(path) })
        }
    }

    impl WorkspaceReader for FakeWorkspace {
        fn metadata(&self, _: &Path) -> FakeWorkspaceFuture<'_, WorkspacePathKind> {
            Box::pin(async { anyhow::bail!("unused fake read") })
        }
        fn read(&self, _: &Path, _: usize) -> FakeWorkspaceFuture<'_, WorkspaceRead> {
            Box::pin(async { anyhow::bail!("unused fake read") })
        }
    }

    impl WorkspaceWriter for FakeWorkspace {
        fn metadata(&self, _: &Path) -> FakeWorkspaceFuture<'_, WorkspacePathKind> {
            Box::pin(async { anyhow::bail!("unused fake write") })
        }
        fn create_dir_all(&self, _: &Path) -> FakeWorkspaceFuture<'_, ()> {
            Box::pin(async { anyhow::bail!("unused fake write") })
        }
        fn write(&self, _: &Path, _: &[u8]) -> FakeWorkspaceFuture<'_, ()> {
            Box::pin(async { anyhow::bail!("unused fake write") })
        }
    }

    impl WorkspaceScanner for FakeWorkspace {
        fn path_kind(&self, _: &Path) -> FakeWorkspaceFuture<'_, WorkspacePathKind> {
            Box::pin(async { anyhow::bail!("unused fake scan") })
        }
        fn read_directory(
            &self,
            _: &Path,
            _: usize,
        ) -> FakeWorkspaceFuture<'_, WorkspaceDirectoryEntries> {
            Box::pin(async { anyhow::bail!("unused fake scan") })
        }
    }

    struct FakeFetcher;

    impl SafeHttpFetcher for FakeFetcher {
        fn fetch(
            &self,
            _: HttpFetchRequest,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<HttpFetchResponse>> + Send + '_>> {
            Box::pin(async { anyhow::bail!("fake fetch must not run during composition") })
        }
    }

    struct FakeRunner;

    impl SubagentRunner for FakeRunner {
        fn run(
            &self,
            _: SubagentTask,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<SubagentReport>> + Send + '_>> {
            Box::pin(async {
                Ok(SubagentReport {
                    report: "unused".to_string(),
                })
            })
        }
    }

    struct FakeInteraction;

    impl UserInteraction for FakeInteraction {
        fn select(&self, _: &str, _: &[String]) -> anyhow::Result<Option<usize>> {
            Ok(None)
        }
        fn readline(&self, _: ContextUsage) -> anyhow::Result<InputOutcome> {
            Ok(InputOutcome::Eof)
        }
    }

    fn fake_workspace() -> WorkspaceCapabilities {
        let workspace = Arc::new(FakeWorkspace);
        WorkspaceCapabilities {
            path_resolver: workspace.clone(),
            reader: workspace.clone(),
            writer: workspace.clone(),
            scanner: workspace,
        }
    }

    use crate::{
        cmd::{cli::Cli, commands::Commands},
        controllers::{
            model_runtime::agent::agent_tool_set::RegisteredToolSet,
            terminal::command_executor::CommandUsageTelemetry,
            workspace::filesystem::WorkspaceCapabilities,
        },
        entities::{
            application::input_outcome::InputOutcome,
            conversation::context_usage::ContextUsage,
            goal::goal_state::GoalState,
            tool::tool_entities::{
                HttpFetchRequest, HttpFetchResponse, PermissionRequirement, SubagentReport,
                SubagentTask,
            },
            workspace::workspace_entities::{
                WorkspaceDirectoryEntries, WorkspacePathKind, WorkspaceRead,
            },
        },
        infrastructure::{
            interaction::UserInteraction,
            tools::{
                goal::GoalCompleter,
                network::SafeHttpFetcher,
                subagent::SubagentRunner,
                workspace::{
                    WorkspacePathResolver, WorkspaceReader, WorkspaceScanner, WorkspaceWriter,
                },
            },
        },
        tools::{
            tool_ask_user::ask_user::AskUser, tool_fetch_url::fetch_url::FetchUrl,
            tool_mark_goal_complete::mark_goal_complete::MarkGoalComplete,
            tool_spawn_subagent::spawn_subagent::SpawnSubagent,
        },
        use_cases::goal::goal_completer::GoalStateCompleter,
    };

    #[tokio::test]
    async fn tool_sets_keep_names_order_permissions_and_shared_goal_capability() -> Result<()> {
        let workspace = fake_workspace();
        let recorder = Arc::new(CommandUsageTelemetry::new());
        let fetcher: Arc<dyn SafeHttpFetcher> = Arc::new(FakeFetcher);
        let mut chat = RegisteredToolSet::new();
        chat.add(AskUser::with_interaction(Arc::new(FakeInteraction)));
        add_workspace_tools(&mut chat, &workspace, recorder.clone()).await?;
        chat.add(FetchUrl::with_fetcher(fetcher.clone()));
        chat.add(SpawnSubagent::with_runner(Arc::new(FakeRunner)));
        let goal_state = Arc::new(GoalState::new());
        let goal: Arc<dyn GoalCompleter> = Arc::new(GoalStateCompleter::new(goal_state));
        let goal_tool = MarkGoalComplete::new(goal.clone());
        ensure!(goal_tool.shares_goal_completer(&goal));
        chat.add(goal_tool);

        let subagent = build_subagent_tools(&workspace, fetcher, recorder).await?;
        assert_registered_tools(
            &chat,
            &[
                ("ask_user", PermissionRequirement::Automatic),
                ("run_command", PermissionRequirement::ConfirmationRequired),
                ("read_file", PermissionRequirement::Automatic),
                ("edit_file", PermissionRequirement::ConfirmationRequired),
                ("write_file", PermissionRequirement::ConfirmationRequired),
                ("search_files", PermissionRequirement::Automatic),
                ("glob_files", PermissionRequirement::Automatic),
                ("fetch_url", PermissionRequirement::Automatic),
                ("spawn_subagent", PermissionRequirement::Automatic),
                ("mark_goal_complete", PermissionRequirement::Automatic),
            ],
        );
        assert_registered_tools(
            &subagent,
            &[
                ("run_command", PermissionRequirement::ConfirmationRequired),
                ("read_file", PermissionRequirement::Automatic),
                ("edit_file", PermissionRequirement::ConfirmationRequired),
                ("write_file", PermissionRequirement::ConfirmationRequired),
                ("search_files", PermissionRequirement::Automatic),
                ("glob_files", PermissionRequirement::Automatic),
                ("fetch_url", PermissionRequirement::Automatic),
            ],
        );
        Ok(())
    }

    fn assert_registered_tools(
        tools: &RegisteredToolSet,
        expected: &[(&str, PermissionRequirement)],
    ) {
        let actual = tools.test_metadata();
        assert_eq!(
            actual,
            expected
                .iter()
                .map(|(name, requirement)| (name.to_string(), *requirement))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn init_cli_values_map_to_application_request() -> Result<()> {
        let Cli {
            command: Commands::Init(args),
        } = Cli::try_parse_from([
            "rigel",
            "init",
            "--base-url",
            "https://model.example/v1",
            "--api-key-env",
            "MODEL_KEY",
        ])?
        else {
            anyhow::bail!("expected init command");
        };
        let request = map_init_request(args);

        ensure!(request.base_url() == "https://model.example/v1");
        ensure!(request.api_key_env() == Some("MODEL_KEY"));
        Ok(())
    }

    #[test]
    fn chat_cli_profile_maps_to_application_request() -> Result<()> {
        let Cli {
            command: Commands::Chat(args),
        } = Cli::try_parse_from(["rigel", "chat", "--profile", "profile.toml"])?
        else {
            anyhow::bail!("expected chat command");
        };
        let request = map_chat_request(args);

        ensure!(
            request
                .profile_path()
                .is_some_and(|path| path == "profile.toml")
        );
        Ok(())
    }
}
