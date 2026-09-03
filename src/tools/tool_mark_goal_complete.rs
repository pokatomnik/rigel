use std::sync::Arc;

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};

use crate::shared::{
    goal::{GoalState, GoalStateError, Report},
    tool_permissions::{PermissionRequirement, ToolPermissionMetadata},
};

use super::contracts::error_codes;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MarkGoalCompleteArgs {
    report: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct MarkGoalCompleteOutput {
    report: String,
}

pub(crate) struct MarkGoalComplete {
    goal_state: Arc<GoalState>,
    permission: PermissionRequirement,
}

impl MarkGoalComplete {
    pub(crate) fn new(goal_state: Arc<GoalState>) -> Self {
        Self {
            goal_state,
            permission: PermissionRequirement::Automatic,
        }
    }

    fn parameters_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "report": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Truthful report of completed work, remaining work, and limitations."
                }
            },
            "required": ["report"]
        })
    }

    fn validate_report(report: &str) -> Result<(), ToolExecutionError> {
        if report.trim().is_empty() {
            return Err(ToolExecutionError::invalid_args(
                "Cannot complete the goal: report must not be empty.",
            )
            .with_code(error_codes::INVALID_ARGUMENT));
        }
        Ok(())
    }

    fn completion_error(error: GoalStateError) -> ToolExecutionError {
        match error {
            GoalStateError::AlreadyActive => ToolExecutionError::other(
                "Cannot complete the goal because the goal state is inconsistent: another goal is active.",
            )
            .with_code(error_codes::GOAL_NOT_ACTIVE),
            GoalStateError::NoActiveGoal => ToolExecutionError::other(
                "Cannot complete the goal: there is no active goal. Use /goal <text> first.",
            )
            .with_code(error_codes::GOAL_NOT_ACTIVE),
        }
    }

    fn output(report: Report) -> MarkGoalCompleteOutput {
        MarkGoalCompleteOutput {
            report: report.as_str().to_string(),
        }
    }
}

impl ToolPermissionMetadata for MarkGoalComplete {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for MarkGoalComplete {
    const NAME: &'static str = "mark_goal_complete";
    type Args = MarkGoalCompleteArgs;
    type Output = MarkGoalCompleteOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Finish the active goal and return a truthful, non-empty work report.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        Self::parameters_schema()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Self::validate_report(&args.report)?;
        let report = self
            .goal_state
            .complete(args.report)
            .map_err(Self::completion_error)?;
        Ok(Self::output(report))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rig::tool::{Tool, ToolContext};

    use super::{MarkGoalComplete, MarkGoalCompleteArgs};
    use crate::{
        shared::{
            goal::GoalState,
            tool_permissions::{PermissionRequirement, ToolPermissionMetadata},
        },
        tools::contracts::error_codes,
    };

    #[test]
    fn schema_requires_only_a_non_empty_report() {
        let schema = MarkGoalComplete::parameters_schema();

        assert_eq!(schema["required"], serde_json::json!(["report"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(schema["properties"]["report"]["minLength"], 1);
        assert_eq!(schema["properties"].as_object().map(|p| p.len()), Some(1));
    }

    #[test]
    fn completion_tool_is_automatic() {
        let tool = MarkGoalComplete::new(Arc::new(GoalState::new()));

        assert_eq!(
            tool.permission_requirement(),
            PermissionRequirement::Automatic
        );
    }

    #[test]
    fn blank_report_is_rejected() {
        let result = MarkGoalComplete::validate_report(" \n\t");

        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code(), Some(error_codes::INVALID_ARGUMENT));
            assert!(error.message().contains("report"));
        }
    }

    #[tokio::test]
    async fn completion_without_active_goal_is_model_visible() {
        let tool = MarkGoalComplete::new(Arc::new(GoalState::new()));
        let mut context = ToolContext::default();

        let result = tool
            .call(
                &mut context,
                MarkGoalCompleteArgs {
                    report: "report".to_string(),
                },
            )
            .await;

        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code(), Some(error_codes::GOAL_NOT_ACTIVE));
            assert!(error.message().contains("active goal"));
        }
    }

    #[tokio::test]
    async fn completion_transitions_state_and_returns_the_report() -> anyhow::Result<()> {
        let state = Arc::new(GoalState::new());
        state.start("inspect repository".to_string())?;
        let tool = MarkGoalComplete::new(state.clone());
        let mut context = ToolContext::default();

        let output = tool
            .call(
                &mut context,
                MarkGoalCompleteArgs {
                    report: "Done with limitations".to_string(),
                },
            )
            .await?;

        assert_eq!(output.report, "Done with limitations");
        assert!(!state.is_active());
        assert_eq!(
            state
                .take_report()
                .map(|report| report.as_str().to_string()),
            Some("Done with limitations".to_string())
        );
        Ok(())
    }
}
