use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::rigel_config::RigelConfig;

#[derive(Deserialize, Serialize, Default)]
pub(super) struct ModelConfig {
    #[serde(default)]
    pub(super) params: toml::Table,
}

const TRANSPORT_OWNED_PARAMS: [&str; 7] = [
    "model",
    "messages",
    "tools",
    "tool_choice",
    "temperature",
    "max_tokens",
    "stream",
];

impl RigelConfig {
    pub(super) fn validate_model_params(&self) -> anyhow::Result<()> {
        for (model_id, model) in self.models_ref() {
            Self::validate_param_names(model_id, &model.params)?;
            serde_json::to_value(&model.params)?;
        }
        Ok(())
    }

    pub(crate) fn model_params(&self, model_id: &str) -> anyhow::Result<Option<Value>> {
        let Some(model) = self.models_ref().get(model_id) else {
            return Ok(None);
        };
        if model.params.is_empty() {
            return Ok(None);
        }
        Self::validate_param_names(model_id, &model.params)?;
        Ok(Some(serde_json::to_value(&model.params)?))
    }

    pub(super) fn merge_models(
        mut global: HashMap<String, ModelConfig>,
        project: HashMap<String, ModelConfig>,
    ) -> HashMap<String, ModelConfig> {
        for (model_id, project_model) in project {
            let merged = match global.remove(&model_id) {
                Some(global_model) => ModelConfig {
                    params: Self::merge_tables(global_model.params, project_model.params),
                },
                None => project_model,
            };
            global.insert(model_id, merged);
        }
        global
    }

    fn validate_param_names(model_id: &str, params: &toml::Table) -> anyhow::Result<()> {
        if let Some(name) = params
            .keys()
            .find(|name| TRANSPORT_OWNED_PARAMS.contains(&name.as_str()))
        {
            anyhow::bail!(
                "model `{model_id}` params cannot override transport-owned field `{name}`"
            );
        }
        Ok(())
    }

    fn merge_tables(mut global: toml::Table, project: toml::Table) -> toml::Table {
        for (key, project_value) in project {
            let merged = match global.remove(&key) {
                Some(global_value) => Self::merge_values(global_value, project_value),
                None => project_value,
            };
            global.insert(key, merged);
        }
        global
    }

    fn merge_values(global: toml::Value, project: toml::Value) -> toml::Value {
        match (global, project) {
            (toml::Value::Table(global), toml::Value::Table(project)) => {
                toml::Value::Table(Self::merge_tables(global, project))
            }
            (toml::Value::Array(mut global), toml::Value::Array(project)) => {
                global.extend(project);
                toml::Value::Array(global)
            }
            (_, project) => project,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::rigel_config::RigelConfig;

    #[test]
    fn exact_model_id_returns_provider_parameters_without_conversion() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config(
            r#"
[models."gpt-5.5".params]
reasoning_effort = "high"
temperature_hint = 0.25
stop = ["END", "DONE"]

[models."gpt-5.5".params.reasoning]
effort = "high"
enabled = true
"#,
        )?;

        assert_eq!(
            config.model_params("gpt-5.5")?,
            Some(serde_json::json!({
                "reasoning_effort": "high",
                "temperature_hint": 0.25,
                "stop": ["END", "DONE"],
                "reasoning": {"effort": "high", "enabled": true}
            }))
        );
        Ok(())
    }

    #[test]
    fn model_lookup_is_exact_and_case_sensitive() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config(
            r#"
[models."Model-A".params]
think = true
"#,
        )?;

        assert!(config.model_params("model-a")?.is_none());
        assert!(config.model_params("Model-A")?.is_some());
        assert!(config.model_params("Model-A-extra")?.is_none());
        Ok(())
    }

    #[test]
    fn transport_owned_parameters_are_rejected() {
        for name in super::TRANSPORT_OWNED_PARAMS {
            let source = format!("[models.unsafe.params]\n{name} = \"configured\"\n");
            assert!(
                RigelConfig::parse_config(source.as_str()).is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn missing_and_empty_parameters_do_not_add_request_parameters() -> anyhow::Result<()> {
        let config = RigelConfig::parse_config(
            r#"
[models."empty".params]
"#,
        )?;

        assert!(config.model_params("empty")?.is_none());
        assert!(config.model_params("missing")?.is_none());
        Ok(())
    }

    #[test]
    fn models_and_params_must_be_tables() {
        assert!(RigelConfig::parse_config("models = []").is_err());
        assert!(RigelConfig::parse_config("[models.test]\nparams = true").is_err());
    }
}
