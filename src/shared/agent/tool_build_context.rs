use std::sync::Arc;

use super::{agent::AgentDependencies, agent_config::AgentConfig};

#[derive(Clone)]
pub(super) struct ToolBuildContext {
    pub(super) config: AgentConfig,
    pub(super) dependencies: Arc<AgentDependencies>,
}
