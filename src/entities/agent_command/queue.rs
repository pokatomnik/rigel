use serde::{Deserialize, Serialize};

/// One user prompt waiting for the current agent turn to finish.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QueuedPrompt {
    /// Stable identifier assigned when the prompt enters the queue.
    id: u64,
    /// Exact prompt text preserved for the eventual submission.
    text: String,
}

impl QueuedPrompt {
    /// Creates a queued prompt from an identifier assigned by the queue.
    pub(crate) fn new(id: u64, text: String) -> Self {
        Self { id, text }
    }

    /// Returns the stable queue identifier.
    pub(crate) const fn id(&self) -> u64 {
        self.id
    }

    /// Returns the exact prompt text.
    pub(crate) fn text(&self) -> &str {
        self.text.as_str()
    }
}

/// FIFO prompt queue owned by the agent workflow and kept only in memory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PromptQueue {
    prompts: Vec<QueuedPrompt>,
    next_id: u64,
}

impl PromptQueue {
    /// Creates an empty queue that does not survive a process restart.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Appends a prompt and returns its assigned identifier.
    pub(crate) fn push(&mut self, text: String) -> u64 {
        self.next_id = self.next_id.saturating_add(1);
        let id = self.next_id;
        self.prompts.push(QueuedPrompt::new(id, text));
        id
    }

    /// Returns queued prompts in submission order.
    pub(crate) fn prompts(&self) -> &[QueuedPrompt] {
        self.prompts.as_slice()
    }

    /// Removes and returns the oldest prompt, if one is waiting.
    pub(crate) fn pop_front(&mut self) -> Option<QueuedPrompt> {
        if self.prompts.is_empty() {
            return None;
        }
        Some(self.prompts.remove(0))
    }

    /// Removes every queued prompt and returns them in their previous order.
    pub(crate) fn clear(&mut self) -> Vec<QueuedPrompt> {
        std::mem::take(&mut self.prompts)
    }

    /// Reports whether no prompts are waiting.
    pub(crate) fn is_empty(&self) -> bool {
        self.prompts.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::PromptQueue;

    #[test]
    fn queue_assigns_increasing_ids_and_preserves_fifo_order() {
        let mut queue = PromptQueue::new();
        let first = queue.push("first".to_string());
        let second = queue.push("second".to_string());

        assert!(first < second);
        assert_eq!(
            queue
                .prompts()
                .iter()
                .map(|prompt| prompt.text())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(queue.pop_front().map(|prompt| prompt.id()), Some(first));
        assert_eq!(queue.pop_front().map(|prompt| prompt.id()), Some(second));
        assert!(queue.pop_front().is_none());
    }

    #[test]
    fn clear_returns_prompts_in_order_and_empties_the_queue() {
        let mut queue = PromptQueue::new();
        queue.push("first".to_string());
        queue.push("second".to_string());

        let cleared = queue.clear();

        assert_eq!(
            cleared
                .iter()
                .map(|prompt| prompt.text())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(queue.is_empty());
    }

    #[test]
    fn ids_keep_increasing_after_the_queue_is_cleared() {
        let mut queue = PromptQueue::new();
        let first = queue.push("first".to_string());
        queue.clear();
        let second = queue.push("second".to_string());

        assert!(second > first);
    }
}
