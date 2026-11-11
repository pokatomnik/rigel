use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Model-specific provider parameters that do not replace transport-owned fields.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub(crate) struct ModelConfig {
    /// Additional JSON-compatible request parameters.
    #[serde(default)]
    pub(crate) params: toml::Table,
}

impl ModelConfig {
    /// Converts configured TOML parameters after validating provider-owned names.
    pub(crate) fn request_parameters(&self, model_id: &str) -> anyhow::Result<Option<Value>> {
        Self::validate_param_names(model_id, &self.params)?;
        if self.params.is_empty() {
            return Ok(None);
        }
        Ok(Some(serde_json::to_value(&self.params)?))
    }

    /// Merges nested provider parameters while letting project values win.
    pub(crate) fn merge(global: Self, project: Self) -> Self {
        Self {
            params: merge_tables(global.params, project.params),
        }
    }

    fn validate_param_names(model_id: &str, params: &toml::Table) -> anyhow::Result<()> {
        const TRANSPORT_FIELDS: [&str; 7] = [
            "model",
            "messages",
            "tools",
            "tool_choice",
            "temperature",
            "max_tokens",
            "stream",
        ];
        if let Some(name) = params
            .keys()
            .find(|name| TRANSPORT_FIELDS.contains(&name.as_str()))
        {
            anyhow::bail!(
                "model `{model_id}` params cannot override transport-owned field `{name}`"
            );
        }
        Ok(())
    }
}

fn merge_tables(mut global: toml::Table, project: toml::Table) -> toml::Table {
    for (key, project_value) in project {
        let merged = match global.remove(&key) {
            Some(global_value) => merge_values(global_value, project_value),
            None => project_value,
        };
        global.insert(key, merged);
    }
    global
}

fn merge_values(global: toml::Value, project: toml::Value) -> toml::Value {
    match (global, project) {
        (toml::Value::Table(global), toml::Value::Table(project)) => {
            toml::Value::Table(merge_tables(global, project))
        }
        (toml::Value::Array(mut global), toml::Value::Array(project)) => {
            global.extend(project);
            toml::Value::Array(global)
        }
        (_, project) => project,
    }
}
