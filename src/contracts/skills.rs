use std::future::Future;
use std::pin::Pin;

use crate::entities::skill::skill::SkillDescriptor;

/// Discovers and loads skills without exposing filesystem paths or directory entries.
pub(crate) trait SkillRepository: Send + Sync {
    /// Returns sorted descriptors with duplicate names already disambiguated for display.
    fn discover(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<SkillDescriptor>>> + Send + '_>>;

    /// Loads instructions by stable name and origin.
    fn instructions(
        &self,
        skill: &SkillDescriptor,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send + '_>>;
}
