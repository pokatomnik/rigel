use std::sync::Arc;

use rig::{agent::AgentBuilder, completion::CompletionModel};
use serde_json::Value;

use crate::entities::{
    configuration::config::RigelConfig, selected_model::selection::SelectedModel,
};

use super::agent_tool_set::AgentToolSet;

#[derive(Clone)]
pub(crate) struct AgentConfig {
    pub(super) base_url: String,
    pub(super) api_key: Option<String>,
    pub(super) tool_set: AgentToolSet,
    pub(super) model_id: Option<String>,
    pub(super) model_context_length: Option<u64>,
    pub(super) system_prompt: Option<String>,
    pub(super) additional_params: Option<Value>,
    config: Arc<RigelConfig>,
}

impl AgentConfig {
    pub(crate) fn new(config: Arc<RigelConfig>) -> Self {
        Self {
            base_url: config.base_url().to_owned(),
            api_key: config
                .api_key_env()
                .and_then(|name| std::env::var(name).ok()),
            tool_set: AgentToolSet::Chat,
            model_id: None,
            model_context_length: None,
            system_prompt: None,
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

    /// Applies one selected model without dropping its provider-reported context limit.
    pub(crate) fn with_selected_model(
        mut self,
        selected_model: SelectedModel,
    ) -> anyhow::Result<Self> {
        self = self.with_model_id(selected_model.id)?;
        self.model_context_length = selected_model.context_length;
        Ok(self)
    }

    /// Stores the effective system prompt that continuation sessions must reuse.
    pub(crate) fn with_system_prompt(mut self, prompt: String) -> Self {
        self.system_prompt = Some(prompt);
        self
    }

    /// Returns the model selected for this session, if one has been assigned.
    pub(crate) fn selected_model(&self) -> Option<SelectedModel> {
        self.model_id
            .clone()
            .map(|id| SelectedModel::new(id, self.model_context_length))
    }

    /// Returns the effective chat system prompt selected for this session.
    pub(crate) fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
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
    use crate::entities::configuration::config::RigelConfig;

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
    fn inherited_model_config_prevents_an_independent_selection() -> anyhow::Result<()> {
        let model = crate::entities::selected_model::selection::SelectedModel::new(
            "parent-model".to_string(),
            Some(4_096),
        );
        let config =
            AgentConfig::new(Arc::new(RigelConfig::default())).with_selected_model(model)?;

        assert_eq!(config.model_id.as_deref(), Some("parent-model"));
        assert_eq!(config.model_context_length, Some(4_096));
        Ok(())
    }

    #[test]
    fn saved_system_prompt_survives_model_and_tool_set_changes() -> anyhow::Result<()> {
        let model = crate::entities::selected_model::selection::SelectedModel::new(
            "model-a".to_string(),
            Some(1_024),
        );
        let config = AgentConfig::new(Arc::new(RigelConfig::default()))
            .with_system_prompt("saved rules".to_string())
            .with_selected_model(model)?
            .with_tool_set(AgentToolSet::Subagent);

        assert_eq!(config.system_prompt(), Some("saved rules"));
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
