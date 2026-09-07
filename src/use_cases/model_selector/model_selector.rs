use std::sync::Arc;

use rig::{
    http_client::{self, HttpClientExt},
    providers::openai,
};
use serde::Deserialize;

use crate::{
    entities::selected_model::SelectedModel,
    shared::{terminal::loader::WithLoader, terminal::terminal_io::TerminalIO},
};

pub(crate) trait ModelSelector {
    async fn select_model(&self, terminal_io: Arc<TerminalIO>) -> anyhow::Result<SelectedModel>;
}

trait ModelFetcher {
    async fn fetch_models(&self) -> anyhow::Result<Vec<SelectedModel>>;
}

impl ModelSelector for openai::Client {
    async fn select_model(&self, terminal_io: Arc<TerminalIO>) -> anyhow::Result<SelectedModel> {
        let models = self.fetch_models().await?;
        if models.is_empty() {
            anyhow::bail!("Empty model list")
        }

        let model = terminal_io.fuzzy_select("Select model", models.as_slice())?;
        Ok(model.clone())
    }
}

impl ModelFetcher for openai::Client {
    async fn fetch_models(&self) -> anyhow::Result<Vec<SelectedModel>> {
        let request = self.get("/models")?.body(http_client::NoBody)?;
        let body = async {
            let response = self.send::<_, Vec<u8>>(request).await?;
            response.into_body().await
        }
        .with_spinner()
        .await?;
        parse_models(body.as_slice())
    }
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
    context_length: Option<u64>,
}

fn parse_models(body: &[u8]) -> anyhow::Result<Vec<SelectedModel>> {
    // Try to parse the models response
    let response = serde_json::from_slice::<ModelsResponse>(body)
        .map_err(|_| anyhow::anyhow!("Incorrect API url"))?;
    Ok(response
        .data
        .into_iter()
        .map(|model| SelectedModel::new(model.id, model.context_length))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::selected_model::SelectedModel;

    #[test]
    fn minimal_model_entries_are_accepted() -> anyhow::Result<()> {
        let models = parse_models(br#"{"data":[{"id":"model-a"}]}"#)?;

        assert_eq!(models, [SelectedModel::new("model-a".to_string(), None)]);
        Ok(())
    }

    #[test]
    fn additional_model_metadata_is_ignored() -> anyhow::Result<()> {
        let body = br#"{"data":[{"id":"model-a","object":"model","owned_by":"owner"}]}"#;
        let models = parse_models(body)?;

        assert_eq!(models, [SelectedModel::new("model-a".to_string(), None)]);
        Ok(())
    }

    #[test]
    fn context_length_is_preserved_when_provider_supplies_it() -> anyhow::Result<()> {
        let body = br#"{"data":[{"id":"model-a","context_length":32768}]}"#;
        let models = parse_models(body)?;

        assert_eq!(models[0].context_length, Some(32_768));
        assert_eq!(models[0].to_string(), "model-a");
        Ok(())
    }

    #[test]
    fn missing_model_id_is_rejected() {
        assert!(parse_models(br#"{"data":[{"object":"model"}]}"#).is_err());
    }

    #[test]
    fn incorrect_api_url_returns_error() {
        // Test with invalid JSON or wrong API response format
        let result = parse_models(b"not json at all");
        assert!(result.is_err());
        assert_eq!(format!("{}", result.unwrap_err()), "Incorrect API url");
    }
}
