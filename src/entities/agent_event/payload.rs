use serde::{Deserialize, Serialize};

/// Where an interactive request originated inside the agent runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum InteractionOrigin {
    /// The main chat agent.
    MainAgent,
    /// An isolated subagent session.
    Subagent,
}

/// One outstanding tool permission decision requested from the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PermissionRequestView {
    /// Correlation identifier used by the eventual reply.
    pub(crate) correlation_id: u64,
    /// Exact runtime tool name.
    pub(crate) tool_name: String,
    /// Complete tool arguments for pretty-printed JSON display.
    pub(crate) arguments: serde_json::Value,
    /// Agent scope that asked for the decision.
    pub(crate) origin: InteractionOrigin,
}

impl PermissionRequestView {
    /// Creates a permission request from provider-neutral runtime data.
    pub(crate) fn new(
        correlation_id: u64,
        tool_name: String,
        arguments: serde_json::Value,
        origin: InteractionOrigin,
    ) -> Self {
        Self {
            correlation_id,
            tool_name,
            arguments,
            origin,
        }
    }
}

/// One free-form question asked by the `ask_user` tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QuestionView {
    /// Correlation identifier used by the eventual answer.
    pub(crate) correlation_id: u64,
    /// Exact question text.
    pub(crate) question: String,
    /// Selectable answers supplied by the model.
    pub(crate) answers: Vec<String>,
    /// Agent scope that asked the question.
    pub(crate) origin: InteractionOrigin,
}

impl QuestionView {
    /// Creates a question view without changing the model-supplied text.
    pub(crate) fn new(
        correlation_id: u64,
        question: String,
        answers: Vec<String>,
        origin: InteractionOrigin,
    ) -> Self {
        Self {
            correlation_id,
            question,
            answers,
            origin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{InteractionOrigin, PermissionRequestView, QuestionView};

    #[test]
    fn permission_view_round_trips_with_json_arguments() -> anyhow::Result<()> {
        let view = PermissionRequestView::new(
            7,
            "run_command".to_string(),
            serde_json::json!({"command": "cargo test"}),
            InteractionOrigin::Subagent,
        );

        let json = serde_json::to_value(&view)?;
        let restored: PermissionRequestView = serde_json::from_value(json)?;

        assert_eq!(restored, view);
        assert_eq!(restored.origin, InteractionOrigin::Subagent);
        Ok(())
    }

    #[test]
    fn question_view_preserves_the_answer_list_order() {
        let view = QuestionView::new(
            9,
            "Pick one".to_string(),
            vec!["first".to_string(), "second".to_string()],
            InteractionOrigin::MainAgent,
        );

        assert_eq!(view.answers, ["first", "second"]);
        assert_eq!(view.question, "Pick one");
    }
}
