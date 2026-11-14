use crate::entities::{
    agent_command::command::AgentCommand,
    agent_event::status::{AgentActivity, McpServerStatus},
    palette::{
        item::{CopyTarget, PaletteAction, PaletteItem},
        level::PaletteLevel,
    },
    render_model::palette::PaletteView,
    selected_model::selection::SelectedModel,
    skill::definition::SkillDescriptor,
};

mod mcp;
mod models;
mod skills;

/// Navigation state of the command palette overlay.
pub(crate) struct PaletteState {
    level: PaletteLevel,
    selected: usize,
    idle: bool,
    mcp: Vec<McpServerStatus>,
    models: Vec<SelectedModel>,
    skills: Vec<SkillDescriptor>,
    current_model: Option<String>,
    read_only: Option<String>,
    notices: usize,
    goal_active: bool,
    status: Option<String>,
}

impl PaletteState {
    /// Opens the palette on its root level for the current agent activity.
    pub(crate) fn root(activity: AgentActivity, mcp: Vec<McpServerStatus>) -> Self {
        let idle = activity.is_idle();
        Self {
            level: PaletteLevel::Root,
            selected: 0,
            idle,
            mcp,
            models: Vec::new(),
            skills: Vec::new(),
            current_model: None,
            read_only: None,
            notices: 0,
            goal_active: false,
            status: (!idle).then(|| "Commands are available when the agent is idle.".to_string()),
        }
    }

    /// Opens the startup palette directly on the model list.
    ///
    /// With no configured models this is the "not configured" state: only exit is available.
    pub(crate) fn startup(models: Vec<SelectedModel>, current_model: Option<String>) -> Self {
        let selected = current_model
            .as_deref()
            .and_then(|current| models.iter().position(|model| model.id == current))
            .unwrap_or(0);
        Self {
            level: PaletteLevel::Models,
            selected,
            idle: true,
            mcp: Vec::new(),
            models,
            skills: Vec::new(),
            current_model,
            read_only: None,
            notices: 0,
            goal_active: false,
            status: None,
        }
    }

    /// Adds the configured model list to a manually opened palette.
    pub(crate) fn with_models(mut self, models: Vec<SelectedModel>) -> Self {
        self.models = models;
        self
    }

    /// Adds the discovered skill list to a manually opened palette.
    pub(crate) fn with_skills(mut self, skills: Vec<SkillDescriptor>) -> Self {
        self.skills = skills;
        self
    }

    /// Adds the currently active model identifier.
    pub(crate) fn with_current_model(mut self, current: Option<String>) -> Self {
        self.current_model = current;
        self
    }

    /// Adds the persistence failure action when durable writes are blocked.
    pub(crate) fn with_read_only(mut self, reason: Option<String>) -> Self {
        self.read_only = reason;
        self
    }

    /// Adds the dismiss action when transient notices are visible.
    pub(crate) fn with_notices(mut self, count: usize) -> Self {
        self.notices = count;
        self
    }

    /// Marks the goal-dependent commands unavailable while a goal is active.
    pub(crate) fn with_goal_active(mut self, active: bool) -> Self {
        self.goal_active = active;
        self
    }

    /// Moves the selection by one item, wrapping at both ends.
    pub(crate) fn move_selection(&mut self, delta: i32) {
        let len = self.items().len();
        if len == 0 {
            return;
        }
        let current = self.selected as i32;
        let next = (current + delta).rem_euclid(len as i32);
        self.selected = next as usize;
    }

    /// Opens a nested level with the first item selected.
    pub(crate) fn open(&mut self, level: PaletteLevel) {
        self.level = level;
        self.selected = 0;
        self.status = None;
    }

    /// Returns to the parent level, or reports that the root was reached.
    pub(crate) fn back(&mut self) -> bool {
        match self.level.parent() {
            Some(parent) => {
                self.level = parent;
                self.selected = 0;
                self.status = None;
                true
            }
            None => false,
        }
    }

    /// Returns the highlighted action, if it is enabled.
    pub(crate) fn selected_action(&self) -> Option<PaletteAction> {
        let items = self.items();
        let item = items.get(self.selected)?;
        item.enabled.then(|| item.action.clone())
    }

    /// Builds the immutable palette snapshot for one frame.
    pub(crate) fn view(&self) -> PaletteView {
        PaletteView::new(self.level, self.items())
            .with_selected(self.selected)
            .with_status(self.status.clone())
    }

    fn items(&self) -> Vec<PaletteItem> {
        match self.level {
            PaletteLevel::Root => self.root_items(),
            PaletteLevel::Models => self.model_items(),
            PaletteLevel::McpServers => self.mcp_items(),
            PaletteLevel::Skills => self.skill_items(),
        }
    }

    fn root_items(&self) -> Vec<PaletteItem> {
        let gated = |item: PaletteItem| {
            if self.idle && !self.goal_active {
                item
            } else {
                item.disabled()
            }
        };
        let mut items = vec![
            gated(self.command_item("compact", "Compact context", AgentCommand::Compact)),
            gated(PaletteItem::new(
                "models",
                "Change model",
                PaletteAction::Open(PaletteLevel::Models),
            )),
        ];
        if !self.mcp.is_empty() {
            items.push(PaletteItem::new(
                "mcp",
                "MCP servers",
                PaletteAction::Open(PaletteLevel::McpServers),
            ));
        }
        items.extend([
            gated(PaletteItem::new(
                "skills",
                "Use a skill…",
                PaletteAction::Open(PaletteLevel::Skills),
            )),
            gated(PaletteItem::new(
                "new",
                "New conversation",
                PaletteAction::ConfirmNewConversation,
            )),
            gated({
                let item = PaletteItem::new("goal", "Set a goal…", PaletteAction::OpenGoal);
                if self.goal_active {
                    item.with_detail("goal is active")
                } else {
                    item
                }
            }),
            self.copy_item("copy-answer", "Copy last answer", CopyTarget::Answer),
            self.copy_item("copy-block", "Copy last block", CopyTarget::Block),
            self.copy_item("copy-summary", "Copy summary", CopyTarget::Summary),
            PaletteItem::new("latest", "Jump to latest", PaletteAction::JumpToLatest),
        ]);
        if let Some(reason) = &self.read_only {
            items.push(
                PaletteItem::new(
                    "retry-persistence",
                    "Retry saving history",
                    PaletteAction::RetryPersistence,
                )
                .with_detail(reason.clone()),
            );
        }
        if self.notices > 0 {
            items.push(PaletteItem::new(
                "dismiss-notices",
                "Dismiss notices",
                PaletteAction::DismissNotices,
            ));
        }
        items.push(PaletteItem::new(
            "exit",
            "Exit Rigel",
            PaletteAction::ConfirmExit,
        ));
        items
    }

    fn command_item(&self, id: &str, label: &str, command: AgentCommand) -> PaletteItem {
        PaletteItem::new(id, label, PaletteAction::Command(command))
    }

    fn copy_item(&self, id: &str, label: &str, target: CopyTarget) -> PaletteItem {
        PaletteItem::new(id, label, PaletteAction::Copy(target))
    }
}

fn back_item() -> PaletteItem {
    PaletteItem::new("back", "← Back", PaletteAction::Back)
}

#[cfg(test)]
mod tests {
    use super::PaletteState;
    use crate::entities::{
        agent_event::status::{AgentActivity, McpServerState, McpServerStatus},
        palette::{
            item::{ItemTone, PaletteAction},
            level::PaletteLevel,
        },
        selected_model::selection::SelectedModel,
    };

    fn model(id: &str, context: u64) -> SelectedModel {
        SelectedModel::new(id.to_string(), Some(context))
    }

    #[test]
    fn root_commands_are_gated_until_the_agent_is_idle() {
        let busy = PaletteState::root(AgentActivity::Running, Vec::new());
        let compact = busy.items().into_iter().find(|item| item.id == "compact");
        assert!(compact.is_some_and(|item| !item.enabled));

        let idle = PaletteState::root(AgentActivity::Idle, Vec::new());
        let compact = idle.items().into_iter().find(|item| item.id == "compact");
        assert!(compact.is_some_and(|item| item.enabled));
    }

    #[test]
    fn active_goal_gates_state_changing_commands() {
        let palette = PaletteState::root(AgentActivity::Idle, Vec::new()).with_goal_active(true);
        let items = palette.items();

        assert!(items.iter().filter(|item| item.id != "exit").all(|item| {
            !matches!(
                item.action,
                PaletteAction::Command(_)
                    | PaletteAction::Open(PaletteLevel::Models)
                    | PaletteAction::Open(PaletteLevel::Skills)
                    | PaletteAction::ConfirmNewConversation
                    | PaletteAction::OpenGoal
            ) || !item.enabled
        }));
    }

    #[test]
    fn startup_opens_the_model_list_with_context_and_current_marker() {
        let palette = PaletteState::startup(vec![model("a", 32_000), model("z", 1_000_000)], None);
        assert_eq!(palette.view().level, PaletteLevel::Models);
        let items = palette.items();

        assert_eq!(items[0].label, "a");
        assert_eq!(items[0].detail.as_deref(), Some("32K"));
        assert!(items[0].detail_right);
        assert!(matches!(items[0].action, PaletteAction::Command(_)));
        assert_eq!(items[1].detail.as_deref(), Some("1M"));
        assert!(matches!(
            items.last().map(|item| &item.action),
            Some(PaletteAction::Back)
        ));
    }

    #[test]
    fn current_model_uses_the_success_tone() {
        let palette = PaletteState::startup(vec![model("a", 32_000)], Some("a".to_string()));
        let item = palette.items().into_iter().next();

        assert!(item.is_some_and(|item| {
            item.tone == ItemTone::Success && item.detail.as_deref() == Some("current · 32K")
        }));
    }

    #[test]
    fn empty_model_list_offers_only_exit() {
        let palette = PaletteState::startup(Vec::new(), None);
        let items = palette.items();

        assert!(items.iter().any(|item| item.id == "models-unconfigured"));
        assert!(items.iter().any(|item| item.id == "exit"));
        assert!(
            !items
                .iter()
                .any(|item| matches!(item.action, PaletteAction::Back))
        );
    }

    #[test]
    fn mcp_item_is_hidden_without_servers_and_colored_with_statuses() {
        let without = PaletteState::root(AgentActivity::Idle, Vec::new());
        assert!(!without.items().iter().any(|item| item.id == "mcp"));

        let palette = PaletteState::root(
            AgentActivity::Idle,
            vec![
                McpServerStatus::new("github".to_string(), McpServerState::Running),
                McpServerStatus::new("broken".to_string(), McpServerState::Failed),
            ],
        );
        assert!(palette.items().iter().any(|item| item.id == "mcp"));
        let mut palette = palette;
        palette.open(PaletteLevel::McpServers);
        let items = palette.items();

        assert!(items.iter().any(|item| item.label == "github"
            && item.tone == ItemTone::Success
            && item.detail.as_deref() == Some("running")));
        assert!(items.iter().any(|item| item.label == "broken"
            && item.tone == ItemTone::Error
            && item.detail.as_deref() == Some("failed to start")));
    }

    #[test]
    fn exit_is_available_even_while_the_agent_runs() {
        let palette = PaletteState::root(AgentActivity::Running, Vec::new());
        let exit = palette.items().into_iter().find(|item| item.id == "exit");

        assert!(matches!(
            exit.map(|item| item.action),
            Some(PaletteAction::ConfirmExit)
        ));
    }
}
