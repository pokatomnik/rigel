use std::{sync::Arc, time::Duration};

use reqwest::Url;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde::Serialize;

use crate::{
    contracts::tools::{network::SafeHttpFetcher, permissions::ToolPermissionMetadata},
    entities::tool::tool_entities::{HttpFetchRequest, PermissionRequirement},
    tools::{action::Action, error_codes},
};

use super::{
    errors::{
        blocked_network_target, fetch_error, invalid_url, invalid_url_credentials,
        invalid_url_host, invalid_url_scheme, timeout_error,
    },
    policy::is_allowed_network_target,
};

const SERVER_TIMEOUT: Duration = Duration::from_secs(20);

const MAX_CONTENT_BYTES: usize = 32 * 1024;
const CONTENT_TRUNCATION_SUFFIX: &str = "\n[content truncated: content is limited; use a more precise URL or a separate follow-up operation]";

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FetchUrlArgs {
    url: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct FetchUrlOutput {
    action: Action,
    ok: bool,
    url: String,
    status: u16,
    content_type: String,
    content: String,
    error_kind: Option<String>,
    error: Option<String>,
    truncated: bool,
}

pub(crate) struct FetchUrl {
    fetcher: Arc<dyn SafeHttpFetcher>,
    permission: PermissionRequirement,
}

impl FetchUrl {
    /// Creates the URL-fetching tool from a composition-root-selected safe fetcher.
    pub(crate) fn with_fetcher(fetcher: Arc<dyn SafeHttpFetcher>) -> Self {
        Self {
            fetcher,
            permission: PermissionRequirement::Automatic,
        }
    }

    fn parse_url(url: &str) -> Result<Url, ToolExecutionError> {
        let parsed = Url::parse(url).map_err(|error| invalid_url(url, error))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(invalid_url_scheme(url));
        }
        if parsed.host_str().is_none() {
            return Err(invalid_url_host(url));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(invalid_url_credentials());
        }
        if !is_allowed_network_target(&parsed) {
            return Err(blocked_network_target());
        }
        Ok(parsed)
    }

    async fn fetch(&self, url: &str) -> Result<FetchUrlOutput, ToolExecutionError> {
        let parsed = Self::parse_url(url)?;
        let request = HttpFetchRequest {
            url: parsed.to_string(),
        };
        let response = self
            .fetcher
            .fetch(request)
            .await
            .map_err(|error| fetch_failure(url, error))?;
        let content_truncated = response.body.ends_with(CONTENT_TRUNCATION_SUFFIX);
        let content = if content_truncated {
            response.body
        } else {
            limit_content(response.body, false).0
        };
        let metadata = (
            url.to_string(),
            "application/octet-stream".to_string(),
            response.status,
        );
        Ok(build_output(metadata, content, content_truncated))
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
        "Fetch a known HTTP or HTTPS URL and return bounded readable text. Use dedicated file tools for workspace files and search; this tool does not discover URLs.".to_string()
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

fn fetch_failure(url: &str, error: anyhow::Error) -> ToolExecutionError {
    match error.downcast::<ToolExecutionError>() {
        Ok(error) => error,
        Err(error) => fetch_error(url, std::io::Error::other(format!("{error:#}"))),
    }
}

fn build_output(
    metadata: (String, String, u16),
    content: String,
    truncated: bool,
) -> FetchUrlOutput {
    let (url, content_type, status) = metadata;
    let (ok, error_kind, error) = http_status_result(status, truncated);
    FetchUrlOutput {
        action: Action::Fetched,
        ok,
        url,
        status,
        content_type,
        content,
        error_kind,
        error,
        truncated,
    }
}

fn http_status_result(status: u16, truncated: bool) -> (bool, Option<String>, Option<String>) {
    if (200..300).contains(&status) {
        return (true, None, None);
    }
    let truncation_hint = if truncated {
        " Content is limited; use a more precise URL or a separate follow-up operation."
    } else {
        ""
    };
    (
        false,
        Some(error_codes::HTTP_STATUS_ERROR_KIND.to_string()),
        Some(format!(
            "The server returned HTTP {status}; do not treat this page as the requested content.{truncation_hint}"
        )),
    )
}

fn limit_content(content: String, source_truncated: bool) -> (String, bool) {
    if !source_truncated && content.len() <= MAX_CONTENT_BYTES {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::model_runtime::http_fetcher::ReqwestSafeHttpFetcher;
    use crate::tools::tool_fetch_url::errors::{fetch_error, timeout_error};

    #[test]
    fn schema_contains_only_url() {
        let tool = FetchUrl::with_fetcher(Arc::new(ReqwestSafeHttpFetcher::new(Arc::new(
            reqwest::Client::new(),
        ))));
        let schema = tool.parameters();

        assert_eq!(schema["required"], serde_json::json!(["url"]));
        assert_eq!(schema["additionalProperties"], serde_json::json!(false));
        assert_eq!(schema["properties"]["url"]["minLength"], 1);
        assert_eq!(
            schema["properties"].as_object().map(|value| value.len()),
            Some(1)
        );
    }

    #[test]
    fn unsupported_scheme_is_rejected_before_network_request() {
        let error = FetchUrl::parse_url("file:///tmp/secret").err();

        assert_eq!(
            error.as_ref().and_then(ToolExecutionError::code),
            Some(error_codes::INVALID_ARGUMENT)
        );
        assert!(error.as_ref().is_some_and(|error| {
            error
                .model_feedback()
                .is_some_and(|message| message.contains("file:///tmp/secret"))
        }));
    }

    #[test]
    fn invalid_url_inputs_are_rejected_before_network_request() {
        for url in [
            "",
            "data:text/plain,secret",
            "ftp://example.com/file",
            "http://",
        ] {
            let error = FetchUrl::parse_url(url).err();
            assert!(error.is_some(), "URL should be rejected: {url:?}");
            assert_eq!(
                error.as_ref().and_then(ToolExecutionError::code),
                Some(error_codes::INVALID_ARGUMENT)
            );
        }
    }

    #[test]
    fn http_statuses_are_structured_without_transport_errors() {
        for (status, expected_ok) in [
            (200, true),
            (204, true),
            (299, true),
            (300, false),
            (302, false),
            (404, false),
            (500, false),
        ] {
            let output = build_output(
                (
                    "https://example.com/final".to_string(),
                    "text/html".to_string(),
                    status,
                ),
                "body".to_string(),
                false,
            );
            assert_eq!(output.ok, expected_ok);
            assert_eq!(output.status, status);
            assert_eq!(output.url, "https://example.com/final");
            if expected_ok {
                assert_eq!(output.error_kind, None);
                assert_eq!(output.error, None);
            } else {
                assert_eq!(
                    output.error_kind.as_deref(),
                    Some(error_codes::HTTP_STATUS_ERROR_KIND)
                );
                assert!(output.error.as_deref().is_some_and(|error| {
                    error.contains(&format!("HTTP {status}"))
                        && error.contains("do not treat this page as the requested content")
                }));
            }
        }
    }

    #[test]
    fn successful_output_serializes_null_error_fields() {
        let output = build_output(
            (
                "https://example.com/final".to_string(),
                "text/html".to_string(),
                200,
            ),
            "body".to_string(),
            false,
        );
        let serialized = serde_json::to_value(output);

        assert!(serialized.is_ok());
        if let Ok(value) = serialized {
            assert_eq!(value["action"], serde_json::json!("fetched"));
            assert_eq!(value["ok"], serde_json::json!(true));
            assert_eq!(value["error_kind"], serde_json::Value::Null);
            assert_eq!(value["error"], serde_json::Value::Null);
            assert_eq!(value["status"], serde_json::json!(200));
        }
    }

    #[test]
    fn http_failure_keeps_body_and_adds_truncation_hint() {
        let output = build_output(
            (
                "https://example.com/missing".to_string(),
                "text/plain".to_string(),
                404,
            ),
            "Not found".to_string(),
            true,
        );

        assert_eq!(output.content, "Not found");
        assert!(!output.ok);
        assert!(output.truncated);
        assert!(output.error.as_deref().is_some_and(|error| {
            error.contains("HTTP 404")
                && error.contains("more precise URL")
                && error.contains("separate follow-up operation")
        }));
    }

    #[test]
    fn truncation_is_utf8_safe_and_actionable() {
        let (content, truncated) = limit_content("я".repeat(MAX_CONTENT_BYTES), false);

        assert!(truncated);
        assert!(content.contains("[content truncated:"));
        assert!(content.contains("more precise URL"));
        assert!(content.contains("separate follow-up operation"));
        assert!(content.is_char_boundary(content.len()));
        assert!(content.len() <= MAX_CONTENT_BYTES);
    }

    #[test]
    fn download_truncation_is_also_reported_in_bounded_content() {
        let (content, truncated) = limit_content("x".repeat(MAX_CONTENT_BYTES), true);

        assert!(truncated);
        assert!(content.contains("[content truncated:"));
        assert!(content.len() <= MAX_CONTENT_BYTES);
    }

    #[test]
    fn private_and_credentialed_targets_are_rejected() {
        for url in [
            "http://127.0.0.1:8080/",
            "http://[::1]/",
            "http://localhost/",
            "http://localhost./",
            "http://127.0.0.1./",
            "https://user:password@example.com/",
        ] {
            let error = FetchUrl::parse_url(url).err();
            assert!(error.is_some(), "target should be rejected: {url}");
            assert_eq!(
                error.as_ref().and_then(ToolExecutionError::code),
                Some(error_codes::INVALID_ARGUMENT)
            );
        }
    }

    #[test]
    fn errors_include_url_and_shared_codes() {
        let fetch = fetch_error("https://example.com/page", std::io::Error::other("offline"));
        let timeout = timeout_error("https://example.com/page");

        assert_eq!(fetch.code(), Some(error_codes::NETWORK_ERROR));
        assert_eq!(timeout.code(), Some(error_codes::TIMEOUT));
        assert!(fetch.message().contains("https://example.com/page"));
    }
}
