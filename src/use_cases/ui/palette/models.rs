use crate::entities::{
    agent_command::command::AgentCommand,
    palette::item::{ItemTone, PaletteAction, PaletteItem},
    render_model::views::format_token_count,
    selected_model::selection::SelectedModel,
};

use super::{PaletteState, back_item};

impl PaletteState {
    pub(super) fn model_items(&self) -> Vec<PaletteItem> {
        if self.models.is_empty() {
            return vec![
                PaletteItem::new(
                    "models-unconfigured",
                    "Rigel is not configured",
                    PaletteAction::Info,
                )
                .disabled()
                .with_detail("Add context_size to a model in the config"),
                PaletteItem::new("exit", "Exit Rigel", PaletteAction::ConfirmExit),
            ];
        }
        let mut items = self
            .models
            .iter()
            .map(|model| self.model_item(model))
            .collect::<Vec<_>>();
        items.push(back_item());
        items
    }

    fn model_item(&self, model: &SelectedModel) -> PaletteItem {
        let context = model
            .context_length
            .map(format_token_count)
            .unwrap_or_else(|| "?".to_string());
        let current = self.current_model.as_deref() == Some(model.id.as_str());
        let detail = if current {
            format!("current · {context}")
        } else {
            context
        };
        let item = PaletteItem::new(
            format!("model-{}", model.id),
            model.id.clone(),
            PaletteAction::Command(AgentCommand::SelectModel {
                model: model.clone(),
            }),
        )
        .with_detail(detail)
        .with_right_detail();
        if current {
            item.with_tone(ItemTone::Success)
        } else {
            item
        }
    }
}
