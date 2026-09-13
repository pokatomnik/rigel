use std::{sync::Arc, time::Duration};

use clap::Parser;
use reqwest::Client;

use crate::{
    adapters::{
        configuration::repository::FileConfigurationRepository,
        mcp_registry::registry::McpRegistry,
        model_runtime::{
            agent::{agent_config::AgentConfig, dependencies::AgentDependencies},
            runtime::chat_runner::RuntimeChatRunner,
        },
        terminal::terminal_io::TerminalIO,
        workspace::filesystem::WorkspaceCapabilities,
    },
    cmd::{cli::Cli, commands::Commands},
    contracts::{
        application::{ApplicationOutput, ChatRunner, ChatWorkflow, ConfigurationInitializer},
        configuration::ConfigurationRepository,
    },
    controllers::{
        chat_controller::chat_controller::ChatController,
        init_controller::init_controller::InitController,
    },
    entities::{
        application::{chat_request::ChatRequest, init_request::InitRequest},
        configuration::configuration::RigelConfig,
        goal::goal_state::GoalState,
    },
    tools::tool_fetch_url::policy::is_allowed_network_target,
    use_cases::{
        chat::chat::Chat, goal::goal_completer::GoalStateCompleter,
        init::initialize::InitializeConfiguration,
    },
};

pub mod adapters;
pub mod cmd;
pub mod contracts;
pub mod controllers;
pub mod entities;
pub mod prompts;
pub mod shared;
pub mod tools;
pub mod use_cases;

const GLOBAL_TOOL_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REDIRECTS: usize = 10;

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
        Commands::Init(args) => {
            let request = InitRequest::new(
                args.base_url().to_string(),
                args.api_key_env().map(ToOwned::to_owned),
            );
            let repository = Arc::new(FileConfigurationRepository::new());
            let initializer: Arc<dyn ConfigurationInitializer> =
                Arc::new(InitializeConfiguration::new(repository));
            InitController::new(initializer).handle(request).await
        }
        Commands::Chat(args) => {
            let request = ChatRequest::new(args.profile_path().map(ToOwned::to_owned));
            let profile_path = request.profile_path();
            let repository = FileConfigurationRepository::new();
            let tool_permission_paths = repository.paths(profile_path)?;
            let config = Arc::new(repository.load(profile_path).await?);
            let http_client = get_http_client().await?;
            let mcp_registry = get_mcp_registry(config.clone()).await?;
            let workspace = get_workspace().await?;
            let goal_state = Arc::new(GoalState::new());
            let goal_completer = Arc::new(GoalStateCompleter::new(goal_state.clone()));
            let dependencies =
                AgentDependencies::new(terminal_io.clone(), http_client, mcp_registry)
                    .with_goal_state(goal_state)
                    .with_goal_completer(goal_completer)
                    .with_tool_permission_paths(tool_permission_paths)
                    .with_workspace(workspace);
            let runtime_runner: Arc<dyn ChatRunner> = Arc::new(RuntimeChatRunner::new(
                AgentConfig::new(config),
                Arc::new(dependencies),
            ));
            let workflow: Arc<dyn ChatWorkflow> = Arc::new(Chat::new(runtime_runner));
            let output: Arc<dyn ApplicationOutput> = terminal_io;
            ChatController::new(workflow, output).handle(request).await
        }
    }
}

async fn get_mcp_registry(config: Arc<RigelConfig>) -> anyhow::Result<Arc<McpRegistry>> {
    Ok(Arc::new(McpRegistry::from_config(config).await?))
}

async fn get_workspace() -> anyhow::Result<Arc<WorkspaceCapabilities>> {
    let current_dir = std::env::current_dir()?;
    let root = tokio::fs::canonicalize(current_dir).await?;
    Ok(Arc::new(WorkspaceCapabilities::new(root)))
}

async fn get_http_client() -> anyhow::Result<Arc<Client>> {
    let config = reqwest::ClientBuilder::new()
        .no_proxy()
        .gzip(true)
        .brotli(true)
        .connect_timeout(GLOBAL_TOOL_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS
                || !is_allowed_network_target(attempt.url())
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .user_agent(include_str!("./user_agents.txt"))
        .build()?;
    Ok(Arc::new(config))
}
