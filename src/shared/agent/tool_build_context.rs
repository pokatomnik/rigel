use std::sync::Arc;

use crate::shared::config::RigelConfig;

use super::agent::AgentDependencies;

#[derive(Clone)]
pub(super) struct ToolBuildContext {
    pub(super) config: Arc<RigelConfig>,
    pub(super) dependencies: Arc<AgentDependencies>,
}
