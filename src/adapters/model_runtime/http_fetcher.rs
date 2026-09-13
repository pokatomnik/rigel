use std::{future::Future, pin::Pin, sync::Arc};

use dom_smoothie::{Config, Readability, TextMode};
use futures::StreamExt;
use reqwest::{Client, Response, Url};

use crate::{
    contracts::tools::network::SafeHttpFetcher,
    entities::tool::tool_entities::{HttpFetchRequest, HttpFetchResponse},
    tools::tool_fetch_url::policy::is_allowed_network_target,
};

const MAX_DOWNLOAD_BYTES: usize = 1024 * 1024;
const MAX_CONTENT_BYTES: usize = 32 * 1024;
const CONTENT_TRUNCATION_SUFFIX: &str = "\n[content truncated: content is limited; use a more precise URL or a separate follow-up operation]";

/// Reqwest-backed safe fetcher assembled by the controller composition boundary.
pub(crate) struct ReqwestSafeHttpFetcher {
    client: Arc<Client>,
}

impl ReqwestSafeHttpFetcher {
    /// Creates a fetcher over the already configured redirect and timeout client.
    pub(crate) fn new(client: Arc<Client>) -> Self {
        Self { client }
    }

    async fn fetch_request(
        client: Arc<Client>,
        request: HttpFetchRequest,
    ) -> anyhow::Result<HttpFetchResponse> {
        let parsed = Url::parse(&request.url)?;
        if !is_allowed_network_target(&parsed) {
            anyhow::bail!("network target is blocked by policy");
        }
        let response = client.get(parsed).send().await?;
        let status = response.status().as_u16();
        let content_type = content_type(&response);
        if !is_allowed_network_target(response.url()) {
            anyhow::bail!("redirect target is blocked by policy");
        }
        let (body, truncated) = read_body(response).await?;
        let readable = readable_content(request.url.as_str(), content_type.as_str(), &body);
        let body = limit_content(readable, truncated);
        Ok(HttpFetchResponse { status, body })
    }
}

impl SafeHttpFetcher for ReqwestSafeHttpFetcher {
    fn fetch(
        &self,
        request: HttpFetchRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<HttpFetchResponse>> + Send + '_>> {
        let client = self.client.clone();
        Box::pin(async move { Self::fetch_request(client, request).await })
    }
}

fn content_type(response: &Response) -> String {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map_or_else(|| "application/octet-stream".to_string(), str::to_string)
}

async fn read_body(response: Response) -> anyhow::Result<(Vec<u8>, bool)> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    let mut truncated = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
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

fn limit_content(content: String, source_truncated: bool) -> String {
    if !source_truncated && content.len() <= MAX_CONTENT_BYTES {
        return content;
    }
    let limit = MAX_CONTENT_BYTES.saturating_sub(CONTENT_TRUNCATION_SUFFIX.len());
    let end = content
        .char_indices()
        .take_while(|(index, _)| *index < limit)
        .last()
        .map_or(0, |(index, character)| index + character.len_utf8());
    format!("{}{}", &content[..end], CONTENT_TRUNCATION_SUFFIX)
}
