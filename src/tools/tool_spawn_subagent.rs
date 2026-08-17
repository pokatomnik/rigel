#![allow(dead_code)]

use std::sync::Arc;

use futures::future::BoxFuture;
use reqwest::Client;
use rig::{
    providers::openai::CompletionModel,
    tool::{Tool, ToolContext, ToolExecutionError},
};
use serde::{Deserialize, Serialize};

use crate::{
    shared::{
        agent::{Agent, AgentConfig, AgentDependencies},
        config::RigelConfig,
        history::{ChatHistory, NonPersistentHistory},
        mcp_registry::McpRegistry,
        terminal::TerminalIO,
        tool_permissions::{PermissionRequirement, ToolPermissionMetadata},
    },
    tools::contracts::error_codes,
    use_cases::subagent::subagent::Subagent,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SpawnSubagentArgs {
    task: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct SpawnSubagentOutput {
    report: String,
}

pub(crate) struct SpawnSubagent {
    subagent: Subagent<CompletionModel>,
    permission: PermissionRequirement,
}

impl SpawnSubagent {
    pub(crate) fn new(
        terminal_io: Arc<TerminalIO>,
        http_client: Arc<Client>,
        config: Arc<RigelConfig>,
    ) -> BoxFuture<'static, anyhow::Result<Self>> {
        Box::pin(async move {
            let subagent = Self::create_subagent(terminal_io, http_client, config).await?;
            Ok(Self {
                subagent,
                permission: PermissionRequirement::Automatic,
            })
        })
    }

    async fn create_subagent(
        terminal_io: Arc<TerminalIO>,
        http_client: Arc<Client>,
        config: Arc<RigelConfig>,
    ) -> anyhow::Result<Subagent<CompletionModel>> {
        let mcp_registry = Arc::new(McpRegistry::from_config(config.clone()).await?);
        let chat_history = Arc::new(ChatHistory::new(Vec::new(), NonPersistentHistory));
        let agent_config = AgentConfig::new(config);
        let dependencies = Arc::new(AgentDependencies::new(
            terminal_io.clone(),
            http_client,
            mcp_registry,
        ));
        let agent = Agent::new_subagent(agent_config, chat_history.clone(), dependencies).await?;
        Ok(Subagent::with_history(agent, terminal_io, chat_history))
    }

    fn parameters_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "task": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Task for the autonomous subagent to complete. Describe what's needs to be done."
                }
            },
            "required": ["task"]
        })
    }

    fn validate_task(task: &str) -> Result<(), ToolExecutionError> {
        if task.trim().is_empty() {
            return Err(ToolExecutionError::invalid_args(
                "Cannot spawn subagent: task must not be empty.",
            )
            .with_code(error_codes::INVALID_ARGUMENT));
        }
        Ok(())
    }

    fn subagent_error(error: anyhow::Error) -> ToolExecutionError {
        let diagnostic = format!("{error:#}");
        ToolExecutionError::other(format!(
            "Cannot spawn subagent: {diagnostic}. Correct the task or provider configuration and retry."
        ))
        .with_code(error_codes::SUBAGENT_ERROR)
        .with_source(std::io::Error::other(diagnostic))
    }
}

impl ToolPermissionMetadata for SpawnSubagent {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for SpawnSubagent {
    const NAME: &'static str = "spawn_subagent";
    type Args = SpawnSubagentArgs;
    type Output = SpawnSubagentOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Run an autonomous subagent for a task and return its structured work report.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        Self::parameters_schema()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Self::validate_task(&args.task)?;
        let report = self
            .subagent
            .run(args.task)
            .await
            .map_err(Self::subagent_error)?;
        Ok(SpawnSubagentOutput { report })
    }
}

#[cfg(test)]
mod tests {
    use super::{SpawnSubagent, SpawnSubagentOutput};

    #[test]
    fn schema_requires_only_task() {
        let schema = SpawnSubagent::parameters_schema();

        assert_eq!(schema["required"], serde_json::json!(["task"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert!(schema["properties"].get("prompt").is_none());
    }

    #[test]
    fn blank_task_is_rejected_with_a_model_visible_error() {
        let error = SpawnSubagent::validate_task(" \n ");

        assert!(error.is_err());
        if let Err(error) = error {
            assert_eq!(
                error.code(),
                Some(crate::tools::contracts::error_codes::INVALID_ARGUMENT)
            );
            assert!(error.message().contains("task"));
        }
    }

    #[test]
    fn subagent_failure_keeps_a_specific_error_code() {
        let error = SpawnSubagent::subagent_error(anyhow::anyhow!("provider unavailable"));

        assert_eq!(
            error.code(),
            Some(crate::tools::contracts::error_codes::SUBAGENT_ERROR)
        );
        assert!(error.message().contains("provider unavailable"));
    }

    #[test]
    fn report_output_serializes_as_a_report() {
        let output = SpawnSubagentOutput {
            report: "SUBAGENT REPORT".to_string(),
        };

        assert_eq!(
            serde_json::to_value(output).ok(),
            Some(serde_json::json!({"report": "SUBAGENT REPORT"}))
        );
    }
}
