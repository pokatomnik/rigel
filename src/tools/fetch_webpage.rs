use std::{sync::Arc, time::Duration};

use dom_smoothie::{Config, Readability, TextMode};
use reqwest::Client;
use rig::tool::{Tool, ToolExecutionError};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct FetchArgs {
    url: String,
    timeout_milliseconds: Option<u64>,
}

pub(crate) struct FetchWebpage {
    client: Arc<reqwest::Client>,
}

impl FetchWebpage {
    pub fn new(client: Arc<Client>) -> Self {
        Self { client }
    }

    async fn fetch_as_html(&self, url: &str) -> Result<String, ToolExecutionError> {
        let result = self.client.get(url).send().await;
        let Ok(result) = result else {
            return Err(
                ToolExecutionError::network("Failed to fetch webpage: {url}")
                    .with_code("FAILED_TO_FETCH"),
            );
        };

        let body = result.text().await;
        let Ok(content) = body else {
            return Err(ToolExecutionError::network(
                "Failed to fetch response body for url: {url}",
            )
            .with_code("FAILED_TO_READ_BODY"));
        };

        Ok(content)
    }

    fn make_readable(&self, url: &str, source: &str) -> Result<String, ToolExecutionError> {
        let config = Config {
            text_mode: TextMode::Markdown,
            ..Default::default()
        };
        let readability = Readability::new(source, Some(url), Some(config));
        let Ok(mut readability) = readability else {
            return Err(ToolExecutionError::invalid_args("Internal tool error")
                .with_code("FETCH_TOOL_INTERNAL_ERROR"));
        };

        let result = readability.parse();
        let Ok(result) = result else {
            return Err(ToolExecutionError::other("Tool failed to parse article")
                .with_code("ARTICLE_PARSE_ERROR"));
        };

        let content: Result<String, _> = result
            .text_content
            .try_into()
            .map_err(|_| anyhow::Error::msg("Failed to extract text"));
        let Ok(content) = content else {
            return Err(
                ToolExecutionError::other("Tool failed to extract text from article")
                    .with_code("TEXT_EXTRACTION_ERROR"),
            );
        };

        Ok(content)
    }

    async fn fetch_readable(&self, url: &str) -> Result<String, ToolExecutionError> {
        let html = self.fetch_as_html(url).await?;
        let contents = self.make_readable(url, html.as_str())?;
        Ok(contents)
    }

    async fn fetch_readable_with_timeout(
        &self,
        url: &str,
        timeout: Option<u64>,
    ) -> Result<String, ToolExecutionError> {
        let Some(timeout) = timeout else {
            return self.fetch_readable(url).await;
        };

        tokio::select! {
            biased;

            result = self.fetch_readable(url) => result.to_owned(),
            _ = tokio::time::sleep(Duration::from_millis(timeout)) => Err(ToolExecutionError::timeout("Tool timed out").with_code("TOOL_TIMED_OUT"))
        }
    }
}

impl Tool for FetchWebpage {
    const NAME: &'static str = "fetch_webpage";
    type Args = FetchArgs;
    type Output = String;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Fetch a webpage by provided URL and get readable text content with optional timeout in milliseconds".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "minLength": 1,
                    "description": "URL of a webpage"
                },
                "timeout_milliseconds": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Maximum time to wait, in milliseconds. Omit to wait without a tool timeout."
                }
            },
            "required": ["url"]
        })
    }

    async fn call(
        &self,
        _: &mut rig::prelude::ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.fetch_readable_with_timeout(args.url.as_str(), args.timeout_milliseconds)
            .await
    }
}
