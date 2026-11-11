use std::{fmt, sync::Mutex};

/// The report produced when an active goal is completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GoalReport(String);

impl GoalReport {
    /// Creates a report while preserving the model's exact text for presentation.
    pub(crate) fn new(report: String) -> Self {
        Self(report)
    }

    /// Returns the report text without changing whitespace or formatting.
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Errors returned when a goal lifecycle transition is not valid.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum GoalStateError {
    /// A second goal was started while another goal is active.
    AlreadyActive,
    /// Completion was requested without an active goal.
    NoActiveGoal,
}

impl fmt::Display for GoalStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyActive => formatter.write_str("another goal is already active"),
            Self::NoActiveGoal => formatter.write_str("there is no active goal"),
        }
    }
}

impl std::error::Error for GoalStateError {}

#[derive(Debug, Default)]
struct GoalMode {
    active_goal: Option<String>,
    report: Option<GoalReport>,
}

/// Owns the one active goal and the report produced when it ends.
pub(crate) struct GoalState {
    mode: Mutex<GoalMode>,
}

impl GoalState {
    /// Creates an empty goal state. The instance is intended to be shared by one session.
    pub(crate) fn new() -> Self {
        Self {
            mode: Mutex::new(GoalMode::default()),
        }
    }

    /// Starts one goal, rejecting a second active goal without changing current state.
    pub(crate) fn start(&self, goal: String) -> Result<(), GoalStateError> {
        let mut mode = self.lock_mode();
        if mode.active_goal.is_some() {
            return Err(GoalStateError::AlreadyActive);
        }
        mode.active_goal = Some(goal);
        mode.report = None;
        Ok(())
    }

    /// Returns the current goal text, if the state is active.
    pub(crate) fn current_goal(&self) -> Option<String> {
        self.lock_mode().active_goal.clone()
    }

    /// Reports whether a goal is currently active.
    pub(crate) fn is_active(&self) -> bool {
        self.lock_mode().active_goal.is_some()
    }

    /// Completes the active goal and retains its report for the next model turn.
    pub(crate) fn complete(&self, report: String) -> Result<GoalReport, GoalStateError> {
        let mut mode = self.lock_mode();
        if mode.active_goal.is_none() {
            return Err(GoalStateError::NoActiveGoal);
        }
        let report = GoalReport::new(report);
        mode.active_goal = None;
        mode.report = Some(report.clone());
        Ok(report)
    }

    /// Takes the pending report exactly once after a successful completion.
    pub(crate) fn take_report(&self) -> Option<GoalReport> {
        self.lock_mode().report.take()
    }

    /// Returns whether the next model call must be forced to produce a final answer.
    pub(crate) fn has_pending_report(&self) -> bool {
        self.lock_mode().report.is_some()
    }

    fn lock_mode(&self) -> std::sync::MutexGuard<'_, GoalMode> {
        match self.mode.lock() {
            Ok(guard) => guard,
            Err(error) => error.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GoalState, GoalStateError};

    #[test]
    fn inactive_state_allows_one_goal() {
        let state = GoalState::new();
        assert!(state.start("first goal".to_string()).is_ok());
        assert!(state.is_active());
        assert_eq!(state.current_goal().as_deref(), Some("first goal"));
    }

    #[test]
    fn active_state_rejects_a_second_goal() {
        let state = GoalState::new();
        assert!(state.start("first goal".to_string()).is_ok());
        assert_eq!(
            state.start("second goal".to_string()),
            Err(GoalStateError::AlreadyActive)
        );
        assert_eq!(state.current_goal().as_deref(), Some("first goal"));
    }

    #[test]
    fn completion_returns_and_stores_the_report() {
        let state = GoalState::new();
        assert!(state.start("first goal".to_string()).is_ok());
        let report = state.complete("partial result and limitations".to_string());
        assert_eq!(
            report.as_ref().map(|report| report.as_str()),
            Ok("partial result and limitations")
        );
        assert!(!state.is_active());
        assert_eq!(
            state.take_report().as_ref().map(|report| report.as_str()),
            Some("partial result and limitations")
        );
        assert!(!state.has_pending_report());
    }

    #[test]
    fn completion_without_an_active_goal_is_rejected() {
        let state = GoalState::new();
        assert_eq!(
            state.complete("report".to_string()),
            Err(GoalStateError::NoActiveGoal)
        );
    }

    #[test]
    fn a_new_goal_can_start_after_report_is_taken() {
        let state = GoalState::new();
        assert!(state.start("first goal".to_string()).is_ok());
        assert!(state.complete("first report".to_string()).is_ok());
        assert!(state.take_report().is_some());
        assert!(state.start("second goal".to_string()).is_ok());
        assert_eq!(state.current_goal().as_deref(), Some("second goal"));
    }
}
