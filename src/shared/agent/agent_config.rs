use std::sync::Arc;

use crate::shared::config::RigelConfig;

use super::agent_tool_set::AgentToolSet;

#[derive(Clone)]
pub(crate) struct AgentConfig {
    pub(super) base_url: String,
    pub(super) api_key: Option<String>,
    pub(super) rigel_config: Arc<RigelConfig>,
    pub(super) tool_set: AgentToolSet,
}

impl AgentConfig {
    pub(crate) fn new(config: Arc<RigelConfig>) -> Self {
        Self {
            base_url: config.base_url().to_owned(),
            api_key: config.api_key(),
            rigel_config: config,
            tool_set: AgentToolSet::Chat,
        }
    }

    pub(super) fn with_tool_set(mut self, tool_set: AgentToolSet) -> Self {
        self.tool_set = tool_set;
        self
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{AgentConfig, AgentToolSet};
    use crate::shared::config::RigelConfig;

    #[test]
    fn agent_config_selects_each_declared_tool_set() {
        let config = AgentConfig::new(Arc::new(
            RigelConfig::default().with_base_url("http://localhost/v1"),
        ));
        assert_eq!(config.tool_set, AgentToolSet::Chat);
        assert_eq!(
            config
                .clone()
                .with_tool_set(AgentToolSet::Subagent)
                .tool_set,
            AgentToolSet::Subagent
        );
        assert_eq!(
            config.with_tool_set(AgentToolSet::Orchestrator).tool_set,
            AgentToolSet::Orchestrator
        );
    }
}
