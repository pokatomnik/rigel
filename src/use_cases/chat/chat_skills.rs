use crate::entities::skill::definition::SkillDescriptor;

use super::workflow::Chat;

impl Chat {
    /// Loads the exact instructions for one palette-selected skill.
    pub(super) async fn skill_instructions(
        &self,
        skill: &SkillDescriptor,
    ) -> anyhow::Result<String> {
        let repository = self
            .skill_repository
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("skill repository was not configured"))?;
        repository.instructions(skill).await
    }
}
