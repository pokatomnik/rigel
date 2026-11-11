use std::sync::Arc;

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};

use crate::{
    entities::{
        selected_model::selection::SelectedModel,
        tool::tool_entities::{PermissionRequirement, SubagentTask},
    },
    infrastructure::tools::{permissions::ToolPermissionMetadata, subagent::SubagentRunner},
    tools::error_codes,
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
    runner: Arc<dyn SubagentRunner>,
    permission: PermissionRequirement,
}

impl SpawnSubagent {
    /// Creates the subagent tool from a runner selected by the composition root.
    ///
    /// The runner owns session construction, isolation, and provider-specific behavior.
    pub(crate) fn with_runner(runner: Arc<dyn SubagentRunner>) -> Self {
        Self {
            runner,
            permission: PermissionRequirement::Automatic,
        }
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

    fn task_from_context(task: String, context: &ToolContext) -> SubagentTask {
        let task = SubagentTask::new(task);
        match context.get::<SelectedModel>().cloned() {
            Some(model) => task.with_selected_model(model),
            None => task,
        }
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
        context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Self::validate_task(&args.task)?;
        let task = Self::task_from_context(args.task, context);
        let report = self.runner.run(task).await.map_err(Self::subagent_error)?;
        Ok(SpawnSubagentOutput {
            report: report.report,
        })
    }
}

#[cfg(test)]
mod tests {
    use rig::tool::ToolContext;

    use super::{SpawnSubagent, SpawnSubagentOutput};
    use crate::entities::selected_model::selection::SelectedModel;

    #[test]
    fn schema_requires_only_task() {
        let schema = SpawnSubagent::parameters_schema();

        assert_eq!(schema["required"], serde_json::json!(["task"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert!(schema["properties"].get("prompt").is_none());
    }

    #[test]
    fn parent_model_context_is_carried_without_changing_tool_schema() {
        let model = SelectedModel::new("parent-model".to_string(), Some(1_024));
        let mut context = ToolContext::new();
        context.insert(model.clone());

        let task = SpawnSubagent::task_from_context("delegate".to_string(), &context);

        assert_eq!(task.selected_model(), Some(&model));
        assert_eq!(
            SpawnSubagent::parameters_schema()["required"],
            serde_json::json!(["task"])
        );
    }

    #[test]
    fn blank_task_is_rejected_with_a_model_visible_error() {
        let error = SpawnSubagent::validate_task(" \n ");

        assert!(error.is_err());
        if let Err(error) = error {
            assert_eq!(
                error.code(),
                Some(crate::tools::error_codes::INVALID_ARGUMENT)
            );
            assert!(error.message().contains("task"));
        }
    }

    #[test]
    fn subagent_failure_keeps_a_specific_error_code() {
        let error = SpawnSubagent::subagent_error(anyhow::anyhow!("provider unavailable"));

        assert_eq!(
            error.code(),
            Some(crate::tools::error_codes::SUBAGENT_ERROR)
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
