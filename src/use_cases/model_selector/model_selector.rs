use std::sync::Arc;

use rig::model::ModelList;

use crate::shared::terminal_io::TerminalIO;

pub(crate) trait ModelSelector {
    fn select_model_sync(&self, terminal_io: Arc<TerminalIO>) -> anyhow::Result<String>;
}

impl ModelSelector for ModelList {
    fn select_model_sync(&self, terminal_io: Arc<TerminalIO>) -> anyhow::Result<String> {
        let model_ids = self.iter().map(|m| m.id.clone()).collect::<Vec<String>>();
        if model_ids.is_empty() {
            anyhow::bail!("Empty model list")
        }

        let model = terminal_io.fuzzy_select("Select model", model_ids.as_slice())?;

        Ok(model.clone())
    }
}
