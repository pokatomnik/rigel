use std::sync::Arc;

use rig::{
    http_client::{self, HttpClientExt},
    providers::openai,
};
use serde::Deserialize;

use crate::shared::terminal_io::TerminalIO;

pub(crate) trait ModelSelector {
    async fn select_model(&self, terminal_io: Arc<TerminalIO>) -> anyhow::Result<String>;
}

trait ModelFetcher {
    async fn fetch_model_ids(&self) -> anyhow::Result<Vec<String>>;
}

impl ModelSelector for openai::Client {
    async fn select_model(&self, terminal_io: Arc<TerminalIO>) -> anyhow::Result<String> {
        let model_ids = self.fetch_model_ids().await?;
        if model_ids.is_empty() {
            anyhow::bail!("Empty model list")
        }

        let model = terminal_io.fuzzy_select("Select model", model_ids.as_slice())?;
        Ok(model.clone())
    }
}

impl ModelFetcher for openai::Client {
    async fn fetch_model_ids(&self) -> anyhow::Result<Vec<String>> {
        let request = self.get("/models")?.body(http_client::NoBody)?;
        let response = self.send::<_, Vec<u8>>(request).await?;
        let body = response.into_body().await?;
        parse_model_ids(body.as_slice())
    }
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

fn parse_model_ids(body: &[u8]) -> anyhow::Result<Vec<String>> {
    let response = serde_json::from_slice::<ModelsResponse>(body)?;
    Ok(response.data.into_iter().map(|model| model.id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_model_entries_are_accepted() -> anyhow::Result<()> {
        let ids = parse_model_ids(br#"{"data":[{"id":"model-a"}]}"#)?;

        assert_eq!(ids, ["model-a"]);
        Ok(())
    }

    #[test]
    fn additional_model_metadata_is_ignored() -> anyhow::Result<()> {
        let body = br#"{"data":[{"id":"model-a","object":"model","owned_by":"owner"}]}"#;
        let ids = parse_model_ids(body)?;

        assert_eq!(ids, ["model-a"]);
        Ok(())
    }

    #[test]
    fn missing_model_id_is_rejected() {
        assert!(parse_model_ids(br#"{"data":[{"object":"model"}]}"#).is_err());
    }
}
