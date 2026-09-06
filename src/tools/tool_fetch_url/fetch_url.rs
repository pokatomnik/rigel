use std::{sync::Arc, time::Duration};

use dom_smoothie::{Config, Readability, TextMode};
use futures::StreamExt;
use reqwest::{Client, Response, Url};
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Serialize;

use crate::shared::tool_permissions::catalog::{PermissionRequirement, ToolPermissionMetadata};

use crate::tools::{action::Action, error_codes};

const SERVER_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_DOWNLOAD_BYTES: usize = 1024 * 1024;
const MAX_CONTENT_BYTES: usize = 32 * 1024;
const CONTENT_TRUNCATION_SUFFIX: &str = "\n[content truncated]";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FetchUrlArgs {
    url: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct FetchUrlOutput {
    action: Action,
    url: String,
    status: u16,
    content_type: String,
    content: String,
    truncated: bool,
}

pub(crate) struct FetchUrl {
    client: Arc<Client>,
    permission: PermissionRequirement,
}

impl FetchUrl {
    pub(crate) fn new(client: Arc<Client>) -> Self {
        Self {
            client,
            permission: PermissionRequirement::Automatic,
        }
    }

    fn parse_url(url: &str) -> Result<Url, ToolExecutionError> {
        let parsed = Url::parse(url).map_err(|error| invalid_url(url, error))?;
        if matches!(parsed.scheme(), "http" | "https") {
            Ok(parsed)
        } else {
            Err(invalid_url_scheme(url))
        }
    }

    async fn fetch(&self, url: &str) -> Result<FetchUrlOutput, ToolExecutionError> {
        let parsed = Self::parse_url(url)?;
        let response = self
            .client
            .get(parsed)
            .send()
            .await
            .map_err(|error| fetch_error(url, error))?;
        let metadata = response_metadata(&response);
        let (body, body_truncated) = read_body(response, url).await?;
        let readable = readable_content(url, &metadata.1, &body);
        let (content, content_truncated) = limit_content(readable);
        Ok(FetchUrlOutput {
            action: Action::Fetched,
            url: metadata.0,
            status: metadata.2,
            content_type: metadata.1,
            content,
            truncated: body_truncated || content_truncated,
        })
    }
}

impl ToolPermissionMetadata for FetchUrl {
    fn permission_requirement(&self) -> PermissionRequirement {
        self.permission
    }
}

impl Tool for FetchUrl {
    const NAME: &'static str = "fetch_url";
    type Args = FetchUrlArgs;
    type Output = FetchUrlOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Fetch an HTTP or HTTPS URL and return readable content.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "url": {
                    "type": "string",
                    "minLength": 1,
                    "description": "HTTP or HTTPS URL to fetch."
                }
            },
            "required": ["url"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        match tokio::time::timeout(SERVER_TIMEOUT, self.fetch(&args.url)).await {
            Ok(result) => result,
            Err(_) => Err(timeout_error(&args.url)),
        }
    }
}

fn response_metadata(response: &Response) -> (String, String, u16) {
    let final_url = response.url().to_string();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map_or_else(|| "application/octet-stream".to_string(), str::to_string);
    (final_url, content_type, response.status().as_u16())
}

async fn read_body(response: Response, url: &str) -> Result<(Vec<u8>, bool), ToolExecutionError> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| response_body_error(url, error))?;
        let remaining = MAX_DOWNLOAD_BYTES.saturating_sub(body.len());
        let kept = chunk.len().min(remaining);
        body.extend_from_slice(&chunk[..kept]);
        truncated |= chunk.len() > kept;
        if truncated {
            break;
        }
    }
    Ok((body, truncated))
}

fn readable_content(url: &str, content_type: &str, body: &[u8]) -> String {
    let source = String::from_utf8_lossy(body).into_owned();
    if !content_type.to_ascii_lowercase().contains("text/html") {
        return source;
    }
    readable_html(url, &source).unwrap_or(source)
}

fn readable_html(url: &str, source: &str) -> Option<String> {
    let config = Config {
        text_mode: TextMode::Markdown,
        ..Default::default()
    };
    let mut readability = Readability::new(source, Some(url), Some(config)).ok()?;
    let result = readability.parse().ok()?;
    Some(result.text_content.into())
}

fn limit_content(content: String) -> (String, bool) {
    if content.len() <= MAX_CONTENT_BYTES {
        return (content, false);
    }
    let limit = MAX_CONTENT_BYTES.saturating_sub(CONTENT_TRUNCATION_SUFFIX.len());
    let end = content
        .char_indices()
        .take_while(|(index, _)| *index < limit)
        .last()
        .map_or(0, |(index, character)| index + character.len_utf8());
    (
        format!("{}{}", &content[..end], CONTENT_TRUNCATION_SUFFIX),
        true,
    )
}

fn invalid_url<E>(url: &str, error: E) -> ToolExecutionError
where
    E: std::error::Error + Send + Sync + 'static,
{
    ToolExecutionError::invalid_args(format!("Cannot fetch URL \"{url}\": URL is invalid."))
        .with_code(error_codes::INVALID_ARGUMENT)
        .with_source(error)
}

fn invalid_url_scheme(url: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot fetch URL \"{url}\": only HTTP and HTTPS URLs are supported."
    ))
    .with_code(error_codes::INVALID_ARGUMENT)
}

fn fetch_error<E>(url: &str, error: E) -> ToolExecutionError
where
    E: std::error::Error + Send + Sync + 'static,
{
    ToolExecutionError::network(format!(
        "Cannot fetch URL \"{url}\": the network request failed."
    ))
    .with_code(error_codes::NETWORK_ERROR)
    .with_source(error)
}

fn response_body_error<E>(url: &str, error: E) -> ToolExecutionError
where
    E: std::error::Error + Send + Sync + 'static,
{
    ToolExecutionError::network(format!(
        "Cannot read the response body for URL \"{url}\": the network request failed."
    ))
    .with_code(error_codes::NETWORK_ERROR)
    .with_source(error)
}

fn timeout_error(url: &str) -> ToolExecutionError {
    ToolExecutionError::timeout(format!(
        "Fetching URL \"{url}\" exceeded the server timeout."
    ))
    .with_code(error_codes::TIMEOUT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_contains_only_url() {
        let tool = FetchUrl::new(Arc::new(Client::new()));
        let schema = tool.parameters();

        assert_eq!(schema["required"], serde_json::json!(["url"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert!(schema["properties"].get("timeout_ms").is_none());
    }

    #[test]
    fn unsupported_scheme_is_rejected_before_network_request() {
        let error = FetchUrl::parse_url("file:///tmp/secret").expect_err("scheme should fail");

        assert_eq!(error.code(), Some(error_codes::INVALID_ARGUMENT));
        assert!(
            error
                .model_feedback()
                .is_some_and(|message| message.contains("file:///tmp/secret"))
        );
    }

    #[test]
    fn errors_include_url_and_shared_codes() {
        let fetch = fetch_error("https://example.com/page", std::io::Error::other("offline"));
        let timeout = timeout_error("https://example.com/page");

        assert_eq!(fetch.code(), Some(error_codes::NETWORK_ERROR));
        assert_eq!(timeout.code(), Some(error_codes::TIMEOUT));
        assert!(fetch.message().contains("https://example.com/page"));
    }

    #[test]
    fn html_content_is_truncated_at_a_utf8_boundary() {
        let (content, truncated) = limit_content("я".repeat(MAX_CONTENT_BYTES));

        assert!(truncated);
        assert!(content.ends_with("[content truncated]"));
        assert!(content.is_char_boundary(content.len()));
    }
}
