use std::sync::Arc;

use rig::{agent::AgentBuilder, completion::CompletionModel};
use serde_json::Value;

use crate::shared::config::rigel_config::RigelConfig;

use super::agent_tool_set::AgentToolSet;

#[derive(Clone)]
pub(crate) struct AgentConfig {
    pub(super) base_url: String,
    pub(super) api_key: Option<String>,
    pub(super) tool_set: AgentToolSet,
    pub(super) model_id: Option<String>,
    pub(super) model_context_length: Option<u64>,
    pub(super) additional_params: Option<Value>,
    config: Arc<RigelConfig>,
}

impl AgentConfig {
    pub(crate) fn new(config: Arc<RigelConfig>) -> Self {
        Self {
            base_url: config.base_url().to_owned(),
            api_key: config.api_key(),
            tool_set: AgentToolSet::Chat,
            model_id: None,
            model_context_length: None,
            additional_params: None,
            config,
        }
    }

    pub(crate) fn with_model_id(mut self, model_id: String) -> anyhow::Result<Self> {
        self.additional_params = self.config.model_params(model_id.as_str())?;
        self.model_id = Some(model_id);
        self.model_context_length = None;
        Ok(self)
    }

    pub(super) fn with_tool_set(mut self, tool_set: AgentToolSet) -> Self {
        self.tool_set = tool_set;
        self
    }

    pub(super) fn apply_additional_params<M>(&self, builder: AgentBuilder<M>) -> AgentBuilder<M>
    where
        M: CompletionModel,
    {
        match &self.additional_params {
            Some(params) => builder.additional_params(params.clone()),
            None => builder,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rig::{AgentBuilder, completion::Prompt, test_utils::MockCompletionModel};

    use super::{AgentConfig, AgentToolSet};
    use crate::shared::config::rigel_config::RigelConfig;

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
    }

    #[test]
    fn agent_config_can_reuse_a_selected_model() -> anyhow::Result<()> {
        let config = AgentConfig::new(Arc::new(RigelConfig::default()))
            .with_model_id("selected-model".to_string())?;

        assert_eq!(config.model_id.as_deref(), Some("selected-model"));
        assert_eq!(config.additional_params, None);
        Ok(())
    }

    #[test]
    fn selected_parameters_are_retained_for_subagent_config() -> anyhow::Result<()> {
        let rigel_config: RigelConfig = toml::from_str(
            r#"
[models."selected-model".params]
think = true
"#,
        )?;
        let config =
            AgentConfig::new(Arc::new(rigel_config)).with_model_id("selected-model".to_string())?;
        let subagent_config = config.clone().with_tool_set(AgentToolSet::Subagent);

        assert_eq!(
            config.additional_params,
            Some(serde_json::json!({"think": true}))
        );
        assert_eq!(subagent_config.additional_params, config.additional_params);
        Ok(())
    }

    #[tokio::test]
    async fn additional_parameters_are_forwarded_to_the_completion_request() -> anyhow::Result<()> {
        let model = MockCompletionModel::text("done");
        let mut config = AgentConfig::new(Arc::new(RigelConfig::default()));
        config.additional_params = Some(serde_json::json!({
            "think": true,
            "reasoning": {"effort": "high"}
        }));
        let agent = config
            .apply_additional_params(AgentBuilder::new(model.clone()))
            .build();

        assert_eq!(agent.prompt("hello").await?, "done");
        let request = model
            .requests()
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("completion request was not captured"))?;
        assert_eq!(
            request.additional_params,
            Some(serde_json::json!({
                "think": true,
                "reasoning": {"effort": "high"}
            }))
        );
        Ok(())
    }
}
