use crate::entities::{
    agent_command::command::AgentCommand,
    palette::item::{PaletteAction, PaletteItem},
};

use super::{PaletteState, back_item};

impl PaletteState {
    /// Builds the skill selection rows, or an explanatory row when none were discovered.
    pub(super) fn skill_items(&self) -> Vec<PaletteItem> {
        if self.skills.is_empty() {
            return vec![
                back_item(),
                PaletteItem::new("skills-empty", "No skills found", PaletteAction::Info).disabled(),
            ];
        }
        let mut items = vec![back_item()];
        items.extend(self.skills.iter().map(|skill| {
            PaletteItem::new(
                format!("skill-{}", skill.name()),
                skill.to_string(),
                PaletteAction::Command(AgentCommand::UseSkill {
                    skill: skill.clone(),
                }),
            )
            .with_detail(skill.origin().label())
        }));
        items
    }
}

#[cfg(test)]
mod tests {
    use super::super::PaletteState;
    use crate::entities::{
        agent_event::status::AgentActivity,
        palette::{item::PaletteAction, level::PaletteLevel},
        skill::definition::{SkillDescriptor, SkillOrigin},
    };

    #[test]
    fn skills_are_listed_with_origin_and_use_action() {
        let mut palette = PaletteState::root(AgentActivity::Idle, Vec::new()).with_skills(vec![
            SkillDescriptor::new("alpha".to_string(), SkillOrigin::Builtin),
        ]);
        palette.open(PaletteLevel::Skills);
        let items = palette.items();

        assert_eq!(items[0].label, "← Back");
        assert_eq!(items[1].label, "alpha");
        assert_eq!(items[1].detail.as_deref(), Some("builtin"));
        assert!(matches!(items[1].action, PaletteAction::Command(_)));
    }

    #[test]
    fn empty_skill_list_offers_back_only() {
        let mut palette = PaletteState::root(AgentActivity::Idle, Vec::new());
        palette.open(PaletteLevel::Skills);
        let items = palette.items();

        assert!(
            items
                .iter()
                .any(|item| item.id == "skills-empty" && !item.enabled)
        );
        assert!(items.iter().any(|item| item.id == "back"));
    }
}
