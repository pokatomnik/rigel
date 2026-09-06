use std::{collections::HashSet, sync::Arc};

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::{Deserialize, Serialize};

use crate::{
    entities::context_usage::ContextUsage,
    shared::{
        goal::goal_state::GoalState,
        terminal::terminal_io::{ReadlineOutcome, TerminalIO},
        tool_permissions::catalog::{PermissionRequirement, ToolPermissionMetadata},
    },
    tools::{action::Action, error_codes},
};

use super::interaction::{InteractionFailure, TerminalInteraction, UserInteraction};

/// The UI-only label appended to every `ask_user` selection menu.
pub(crate) const SOMETHING_ELSE: &str = "Something else";
const MAX_QUESTION_BYTES: usize = 4_096;
const MAX_ANSWER_BYTES: usize = 1_024;
const MAX_ANSWERS: usize = 10;

/// Arguments for asking the user one bounded single-choice question.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AskUserArgs {
    question: String,
    answers: Vec<String>,
}

/// Structured result returned after the user answers the question.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct AskUserOutput {
    action: Action,
    ok: bool,
    answer: String,
}

/// Asks the user one model-authored question without entering a new chat turn.
pub(crate) struct AskUser {
    interaction: Arc<dyn UserInteraction>,
    goal_state: Arc<GoalState>,
    permission: PermissionRequirement,
}

impl AskUser {
    /// Creates the production user-question tool for the current chat session.
    pub(crate) fn new(terminal_io: Arc<TerminalIO>, goal_state: Arc<GoalState>) -> Self {
        Self {
            interaction: Arc::new(TerminalInteraction::new(terminal_io)),
            goal_state,
            permission: PermissionRequirement::Automatic,
        }
    }

    #[cfg(test)]
    fn with_interaction(interaction: Arc<dyn UserInteraction>, goal_state: Arc<GoalState>) -> Self {
        Self {
            interaction,
            goal_state,
            permission: PermissionRequirement::Automatic,
        }
    }

    fn validate_arguments(args: &AskUserArgs) -> Result<(), ToolExecutionError> {
        validate_text("question", &args.question, MAX_QUESTION_BYTES)?;
        if args.answers.is_empty() || args.answers.len() > MAX_ANSWERS {
            return Err(invalid_argument(format!(
                "answers must contain between 1 and {MAX_ANSWERS} items."
            )));
        }
        let mut seen = HashSet::new();
        for answer in &args.answers {
            validate_text("answer", answer, MAX_ANSWER_BYTES)?;
            if answer == SOMETHING_ELSE {
                return Err(invalid_argument(format!(
                    "answer must not use the reserved label `{SOMETHING_ELSE}`."
                )));
            }
            if !seen.insert(answer) {
                return Err(invalid_argument("answers must be unique."));
            }
        }
        Ok(())
    }

    fn answer(
        &self,
        args: &AskUserArgs,
        context: &ToolContext,
    ) -> Result<String, ToolExecutionError> {
        if self.goal_state.is_active() {
            return Err(interaction_unavailable(
                "ask_user is unavailable while an autonomous goal is active.",
            ));
        }
        let items = menu_items(&args.answers);
        let selection = self
            .interaction
            .select(&args.question, &items)
            .map_err(interaction_error)?;
        match selection {
            Some(index) if index < args.answers.len() => Ok(args.answers[index].clone()),
            Some(index) if index == args.answers.len() => self.free_answer(context),
            Some(_) => Err(interaction_unavailable(
                "the selection UI returned an invalid item.",
            )),
            None => Err(user_cancelled("the user cancelled the question.")),
        }
    }

    fn free_answer(&self, context: &ToolContext) -> Result<String, ToolExecutionError> {
        let context_usage = context_usage(context)?;
        loop {
            match self
                .interaction
                .readline(context_usage)
                .map_err(interaction_error)?
            {
                ReadlineOutcome::Text(answer) if answer.trim().is_empty() => continue,
                ReadlineOutcome::Text(answer) => {
                    validate_text("answer", &answer, MAX_ANSWER_BYTES)?;
                    return Ok(answer);
                }
                ReadlineOutcome::Eof => {
                    return Err(interaction_unavailable("the user input reached EOF."));
                }
                ReadlineOutcome::Cancelled => {
                    return Err(user_cancelled("the user cancelled free-form input."));
                }
            }
        }
    }

    fn output(answer: String) -> AskUserOutput {
        AskUserOutput {
            action: Action::Answered,
            ok: true,
            answer,
        }
    }

    fn parameters_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "question": {
                    "type": "string",
                    "minLength": 1,
                    "description": "One concise question whose answer is required to continue."
                },
                "answers": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 10,
                    "uniqueItems": true,
                    "items": {
                        "type": "string",
                        "minLength": 1,
                        "description": "One concise candidate answer selected by the model."
                    },
                    "description": "Candidate answers selected by the model for this question. The tool always appends `Something else` as the final menu item."
                }
            },
            "required": ["question", "answers"]
        })
    }
}

impl ToolPermissionMetadata for AskUser {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for AskUser {
    const NAME: &'static str = "ask_user";
    type Args = AskUserArgs;
    type Output = AskUserOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Ask the user one concise question with one selectable candidate answer or a Something else free-form answer. Use only when an important choice cannot be made reliably from context; the tool waits for the answer inside this call and is unavailable during /goal.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        Self::parameters_schema()
    }

    async fn call(
        &self,
        context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        Self::validate_arguments(&args)?;
        let answer = self.answer(&args, context)?;
        Ok(Self::output(answer))
    }
}

fn menu_items(answers: &[String]) -> Vec<String> {
    let mut items = answers.to_vec();
    items.push(SOMETHING_ELSE.to_string());
    items
}

fn validate_text(field: &str, value: &str, max_bytes: usize) -> Result<(), ToolExecutionError> {
    if value.trim().is_empty() {
        return Err(invalid_argument(format!("{field} must not be empty.")));
    }
    if value.len() > max_bytes {
        return Err(invalid_argument(format!(
            "{field} must not exceed {max_bytes} bytes."
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(invalid_argument(format!(
            "{field} must not contain control characters."
        )));
    }
    Ok(())
}

fn context_usage(context: &ToolContext) -> Result<ContextUsage, ToolExecutionError> {
    context.get::<ContextUsage>().copied().ok_or_else(|| {
        interaction_unavailable("current context usage was not provided to ask_user.")
    })
}

fn invalid_argument(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::invalid_args(message).with_code(error_codes::INVALID_ARGUMENT)
}

fn interaction_error(failure: InteractionFailure) -> ToolExecutionError {
    match failure {
        InteractionFailure::Cancelled => user_cancelled("the user cancelled the interaction."),
        InteractionFailure::Unavailable => {
            interaction_unavailable("the user interaction is unavailable.")
        }
    }
}

fn user_cancelled(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::cancelled(message).with_code(error_codes::USER_CANCELLED)
}

fn interaction_unavailable(message: impl Into<String>) -> ToolExecutionError {
    ToolExecutionError::other(message).with_code(error_codes::INTERACTION_UNAVAILABLE)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use rig::tool::{Tool, ToolContext};

    use super::super::interaction::{InteractionFailure, UserInteraction};
    use super::{AskUser, AskUserArgs, SOMETHING_ELSE};
    use crate::shared::terminal::terminal_io::ReadlineOutcome;
    use crate::{
        entities::context_usage::ContextUsage,
        shared::{
            goal::goal_state::GoalState,
            tool_permissions::catalog::{PermissionRequirement, ToolPermissionMetadata},
        },
        tools::error_codes,
    };

    struct StubInteraction {
        selections: Mutex<VecDeque<Result<Option<usize>, InteractionFailure>>>,
        inputs: Mutex<VecDeque<Result<ReadlineOutcome, InteractionFailure>>>,
        items: Mutex<Vec<String>>,
        contexts: Mutex<Vec<ContextUsage>>,
    }

    impl StubInteraction {
        fn new(
            selection: Result<Option<usize>, InteractionFailure>,
            inputs: Vec<Result<ReadlineOutcome, InteractionFailure>>,
        ) -> Self {
            Self {
                selections: Mutex::new(VecDeque::from([selection])),
                inputs: Mutex::new(inputs.into()),
                items: Mutex::new(Vec::new()),
                contexts: Mutex::new(Vec::new()),
            }
        }

        fn items(&self) -> Option<Vec<String>> {
            self.items.lock().ok().map(|items| items.clone())
        }

        fn contexts(&self) -> Option<Vec<ContextUsage>> {
            self.contexts.lock().ok().map(|contexts| contexts.clone())
        }

        fn input_count(&self) -> Option<usize> {
            self.contexts.lock().ok().map(|contexts| contexts.len())
        }
    }

    impl UserInteraction for StubInteraction {
        fn select(
            &self,
            _question: &str,
            items: &[String],
        ) -> Result<Option<usize>, InteractionFailure> {
            if let Ok(mut captured) = self.items.lock() {
                *captured = items.to_vec();
            }
            self.selections
                .lock()
                .ok()
                .and_then(|mut selections| selections.pop_front())
                .unwrap_or(Err(InteractionFailure::Unavailable))
        }

        fn readline(
            &self,
            context_usage: ContextUsage,
        ) -> Result<ReadlineOutcome, InteractionFailure> {
            if let Ok(mut contexts) = self.contexts.lock() {
                contexts.push(context_usage);
            }
            self.inputs
                .lock()
                .ok()
                .and_then(|mut inputs| inputs.pop_front())
                .unwrap_or(Err(InteractionFailure::Unavailable))
        }
    }

    fn context() -> ToolContext {
        let mut context = ToolContext::new();
        context.insert(ContextUsage::new(Some(800), Some(1_000)));
        context
    }

    fn tool(interaction: Arc<StubInteraction>) -> AskUser {
        AskUser::with_interaction(interaction, Arc::new(GoalState::new()))
    }

    fn args(question: &str, answers: &[&str]) -> AskUserArgs {
        AskUserArgs {
            question: question.to_string(),
            answers: answers.iter().map(|answer| (*answer).to_string()).collect(),
        }
    }

    #[test]
    fn schema_matches_the_single_question_contract() {
        let tool = tool(Arc::new(StubInteraction::new(Ok(Some(0)), Vec::new())));
        let schema = tool.parameters();

        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["required"],
            serde_json::json!(["question", "answers"])
        );
        assert_eq!(schema["properties"]["question"]["minLength"], 1);
        assert_eq!(schema["properties"]["answers"]["minItems"], 1);
        assert_eq!(schema["properties"]["answers"]["maxItems"], 10);
        assert_eq!(schema["properties"]["answers"]["uniqueItems"], true);
    }

    #[test]
    fn automatic_permission_does_not_add_a_confirmation_step() {
        let tool = tool(Arc::new(StubInteraction::new(Ok(Some(0)), Vec::new())));

        assert_eq!(
            tool.permission_requirement(),
            PermissionRequirement::Automatic
        );
    }

    #[tokio::test]
    async fn selected_answer_is_exact_and_does_not_read_freeform_input() -> anyhow::Result<()> {
        let interaction = Arc::new(StubInteraction::new(Ok(Some(1)), Vec::new()));
        let tool = tool(interaction.clone());
        let mut context = ToolContext::new();

        let output = tool
            .call(&mut context, args("Choose a backend", &["local", "remote"]))
            .await?;

        assert_eq!(output.answer, "remote");
        assert_eq!(output.action, crate::tools::action::Action::Answered);
        assert!(output.ok);
        assert_eq!(
            serde_json::to_value(&output)?,
            serde_json::json!({"action": "answered", "ok": true, "answer": "remote"})
        );
        assert_eq!(interaction.input_count(), Some(0));
        assert_eq!(
            interaction.items(),
            Some(vec![
                "local".to_string(),
                "remote".to_string(),
                SOMETHING_ELSE.to_string()
            ])
        );
        Ok(())
    }

    #[tokio::test]
    async fn something_else_retries_empty_input_and_returns_the_same_tool_result_shape()
    -> anyhow::Result<()> {
        let usage = ContextUsage::new(Some(800), Some(1_000));
        let interaction = Arc::new(StubInteraction::new(
            Ok(Some(2)),
            vec![
                Ok(ReadlineOutcome::Text(String::new())),
                Ok(ReadlineOutcome::Text(
                    "  use the remote backend  ".to_string(),
                )),
            ],
        ));
        let tool = tool(interaction.clone());
        let mut context = ToolContext::new();
        context.insert(usage);

        let output = tool
            .call(&mut context, args("Choose a backend", &["local", "remote"]))
            .await?;

        assert_eq!(output.answer, "  use the remote backend  ");
        assert!(output.ok);
        assert_eq!(interaction.input_count(), Some(2));
        assert_eq!(interaction.contexts(), Some(vec![usage, usage]));
        assert!(!serde_json::to_string(&output)?.contains(SOMETHING_ELSE));
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_eof_and_interaction_errors_are_not_successes() {
        for (outcome, expected_code) in [
            (
                Err(InteractionFailure::Cancelled),
                error_codes::USER_CANCELLED,
            ),
            (Ok(None), error_codes::USER_CANCELLED),
        ] {
            let interaction = Arc::new(StubInteraction::new(outcome, Vec::new()));
            let tool = tool(interaction);
            let mut context = ToolContext::new();
            let result = tool.call(&mut context, args("Choose", &["first"])).await;

            assert!(result.is_err());
            if let Err(error) = result {
                assert_eq!(error.code(), Some(expected_code));
            }
        }

        let interaction = Arc::new(StubInteraction::new(
            Ok(Some(1)),
            vec![Ok(ReadlineOutcome::Eof)],
        ));
        let tool = tool(interaction);
        let mut context = context();
        let result = tool.call(&mut context, args("Choose", &["first"])).await;

        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code(), Some(error_codes::INTERACTION_UNAVAILABLE));
        }
    }

    #[test]
    fn invalid_arguments_reject_bounds_controls_duplicates_and_reserved_label() {
        let cases = [
            args(" ", &["first"]),
            args("question", &[]),
            args("question", &[SOMETHING_ELSE]),
            args("question", &["same", "same"]),
            args("question\nwith newline", &["first"]),
            args("question", &["line\u{1b}[31m"]),
            args(&"q".repeat(4_097), &["first"]),
            args("question", &[&"a".repeat(1_025)]),
        ];
        for args in cases {
            let result = AskUser::validate_arguments(&args);
            assert!(result.is_err());
            if let Err(error) = result {
                assert_eq!(error.code(), Some(error_codes::INVALID_ARGUMENT));
            }
        }
        let many_answers = AskUserArgs {
            question: "question".to_string(),
            answers: (0..11).map(|index| format!("answer-{index}")).collect(),
        };
        let result = AskUser::validate_arguments(&many_answers);
        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code(), Some(error_codes::INVALID_ARGUMENT));
        }
    }

    #[tokio::test]
    async fn freeform_cancellation_is_a_distinct_error() {
        let interaction = Arc::new(StubInteraction::new(
            Ok(Some(1)),
            vec![Ok(ReadlineOutcome::Cancelled)],
        ));
        let tool = tool(interaction);
        let mut context = context();

        let result = tool.call(&mut context, args("Choose", &["first"])).await;

        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code(), Some(error_codes::USER_CANCELLED));
        }
    }

    #[tokio::test]
    async fn active_goal_rejects_user_input_before_opening_the_ui() {
        let interaction = Arc::new(StubInteraction::new(Ok(Some(0)), Vec::new()));
        let state = Arc::new(GoalState::new());
        assert!(state.start("work autonomously".to_string()).is_ok());
        let tool = AskUser::with_interaction(interaction.clone(), state);
        let mut context = ToolContext::new();

        let result = tool.call(&mut context, args("Choose", &["first"])).await;

        assert!(result.is_err());
        if let Err(error) = result {
            assert_eq!(error.code(), Some(error_codes::INTERACTION_UNAVAILABLE));
        }
        assert_eq!(interaction.items(), Some(Vec::new()));
        assert_eq!(interaction.input_count(), Some(0));
    }
}
