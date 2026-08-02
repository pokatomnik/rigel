use crate::use_cases::chat::tool_recovery::ToolRecoveryStatus;

#[derive(Default)]
pub(crate) struct StreamOutputState {
    showing_reasoning: bool,
    received_reasoning: bool,
    received_answer: bool,
    received_tool_result: bool,
    pending_tool_recovery: bool,
}

impl StreamOutputState {
    #[cfg(test)]
    pub fn new(
        showing_reasoning: bool,
        received_reasoning: bool,
        received_answer: bool,
    ) -> StreamOutputState {
        Self {
            showing_reasoning,
            received_reasoning,
            received_answer,
            received_tool_result: false,
            pending_tool_recovery: false,
        }
    }

    pub fn showing_reasoning(&self) -> bool {
        self.showing_reasoning
    }

    pub fn set_showing_reasoning(&mut self, showing_reasoning: bool) {
        self.showing_reasoning = showing_reasoning;
    }

    pub fn set_received_reasoning(&mut self, received_reasoning: bool) {
        self.received_reasoning = received_reasoning;
    }

    pub fn set_received_answer(&mut self, received_answer: bool) {
        self.received_answer = received_answer;
        if received_answer {
            self.pending_tool_recovery = false;
        }
    }

    pub fn record_tool_result(&mut self, status: ToolRecoveryStatus) {
        self.received_tool_result = true;

        match status {
            ToolRecoveryStatus::Error => self.pending_tool_recovery = true,
            ToolRecoveryStatus::Recovered => self.pending_tool_recovery = false,
            ToolRecoveryStatus::None => {}
        }
    }

    pub fn requires_tool_recovery(&self) -> bool {
        self.pending_tool_recovery && !self.received_answer
    }

    pub fn requires_answer_recovery(&self) -> bool {
        (self.received_reasoning || self.received_tool_result)
            && !self.received_answer
            && !self.pending_tool_recovery
    }
}

#[cfg(test)]
mod tests {
    use super::StreamOutputState;
    use crate::use_cases::chat::tool_recovery::ToolRecoveryStatus;

    #[test]
    fn reasoning_without_answer_requires_recovery() {
        let state = StreamOutputState::new(false, true, false);
        assert!(state.requires_answer_recovery());
    }

    #[test]
    fn reasoning_with_answer_does_not_require_recovery() {
        let state = StreamOutputState::new(false, true, true);
        assert!(!state.requires_answer_recovery());
    }

    #[test]
    fn missing_reasoning_does_not_require_recovery() {
        assert!(!StreamOutputState::default().requires_answer_recovery());
    }

    #[test]
    fn tool_error_requires_follow_up_action() {
        let mut state = StreamOutputState::default();
        state.record_tool_result(ToolRecoveryStatus::Error);
        assert!(state.requires_tool_recovery());
        assert!(!state.requires_answer_recovery());
    }

    #[test]
    fn final_answer_after_tool_error_completes_recovery() {
        let mut state = StreamOutputState::default();
        state.record_tool_result(ToolRecoveryStatus::Error);
        state.set_received_answer(true);
        assert!(!state.requires_tool_recovery());
        assert!(!state.requires_answer_recovery());
    }

    #[test]
    fn successful_correction_requires_final_answer() {
        let mut state = StreamOutputState::default();
        state.record_tool_result(ToolRecoveryStatus::Error);
        state.record_tool_result(ToolRecoveryStatus::Recovered);
        assert!(!state.requires_tool_recovery());
        assert!(state.requires_answer_recovery());
    }

    #[test]
    fn tool_result_without_answer_requires_answer_recovery() {
        let mut state = StreamOutputState::default();
        state.record_tool_result(ToolRecoveryStatus::None);
        assert!(state.requires_answer_recovery());
    }
}
