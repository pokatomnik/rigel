use crate::entities::{
    agent_event::status::McpServerState,
    palette::item::{ItemTone, PaletteAction, PaletteItem},
};

use super::{PaletteState, back_item};

impl PaletteState {
    pub(super) fn mcp_items(&self) -> Vec<PaletteItem> {
        if self.mcp.is_empty() {
            return vec![
                back_item(),
                PaletteItem::new(
                    "mcp-empty",
                    "No MCP servers are configured",
                    PaletteAction::Back,
                )
                .disabled(),
            ];
        }
        let mut items = vec![back_item()];
        items.extend(self.mcp.iter().map(|status| {
            let (detail, tone) = match status.state {
                McpServerState::Running => ("running", ItemTone::Success),
                McpServerState::Starting => ("starting", ItemTone::Muted),
                McpServerState::Failed => ("failed to start", ItemTone::Error),
            };
            PaletteItem::new(
                format!("mcp-{}", status.name),
                status.name.clone(),
                PaletteAction::Info,
            )
            .with_detail(detail)
            .with_tone(tone)
        }));
        items
    }
}
