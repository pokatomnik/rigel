use std::sync::Arc;

use super::{agent_config::AgentConfig, dependencies::AgentDependencies};

#[derive(Clone)]
pub(super) struct ToolBuildContext {
    pub(super) config: AgentConfig,
    pub(super) dependencies: Arc<AgentDependencies>,
}
