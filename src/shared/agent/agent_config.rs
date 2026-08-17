use std::sync::Arc;

use crate::shared::config::RigelConfig;

use super::agent_tool_set::AgentToolSet;

#[derive(Clone)]
pub(crate) struct AgentConfig {
    pub(super) base_url: String,
    pub(super) api_key: Option<String>,
    pub(super) tool_set: AgentToolSet,
    pub(super) model_id: Option<String>,
}

impl AgentConfig {
    pub(crate) fn new(config: Arc<RigelConfig>) -> Self {
        Self {
            base_url: config.base_url().to_owned(),
            api_key: config.api_key(),
            tool_set: AgentToolSet::Chat,
            model_id: None,
        }
    }

    pub(crate) fn with_model_id(mut self, model_id: String) -> Self {
        self.model_id = Some(model_id);
        self
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

    #[test]
    fn agent_config_can_reuse_a_selected_model() {
        let config = AgentConfig::new(Arc::new(RigelConfig::default()))
            .with_model_id("selected-model".to_string());

        assert_eq!(config.model_id.as_deref(), Some("selected-model"));
    }
}
