#[derive(Default)]
pub(crate) struct StreamOutputState {
    showing_reasoning: bool,
    received_reasoning: bool,
    received_answer: bool,
}

impl StreamOutputState {
    #[cfg(test)]
    pub fn new(
        showing_reasoning: bool,
        received_reasoning: bool,
        received_answer: bool,
    ) -> StreamOutputState {
        Self {
            showing_reasoning: showing_reasoning,
            received_reasoning: received_reasoning,
            received_answer: received_answer,
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
    }

    pub fn requires_answer_recovery(&self) -> bool {
        self.received_reasoning && !self.received_answer
    }
}
