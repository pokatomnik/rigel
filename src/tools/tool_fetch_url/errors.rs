use std::error::Error;

use rig::tool::ToolExecutionError;

use crate::tools::error_codes;

pub(super) fn invalid_url<E>(url: &str, error: E) -> ToolExecutionError
where
    E: Error + Send + Sync + 'static,
{
    ToolExecutionError::invalid_args(format!("Cannot fetch URL \"{url}\": URL is invalid."))
        .with_code(error_codes::INVALID_ARGUMENT)
        .with_source(error)
}

pub(super) fn invalid_url_host(url: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot fetch URL \"{url}\": an absolute HTTP or HTTPS URL must include a host."
    ))
    .with_code(error_codes::INVALID_ARGUMENT)
}

pub(super) fn invalid_url_credentials() -> ToolExecutionError {
    ToolExecutionError::invalid_args(
        "Cannot fetch URL: credentials in the URL are not supported; remove user information and retry.",
    )
    .with_code(error_codes::INVALID_ARGUMENT)
}

pub(super) fn blocked_network_target() -> ToolExecutionError {
    ToolExecutionError::invalid_args(
        "Cannot fetch this URL: the target is blocked by the network policy. Use a public HTTP or HTTPS URL.",
    )
    .with_code(error_codes::INVALID_ARGUMENT)
}

pub(super) fn blocked_redirect_target() -> ToolExecutionError {
    ToolExecutionError::network(
        "Cannot fetch this URL: a redirect target was blocked by the network policy.",
    )
    .with_code(error_codes::NETWORK_ERROR)
}

pub(super) fn invalid_url_scheme(url: &str) -> ToolExecutionError {
    ToolExecutionError::invalid_args(format!(
        "Cannot fetch URL \"{url}\": only HTTP and HTTPS URLs are supported."
    ))
    .with_code(error_codes::INVALID_ARGUMENT)
}

pub(super) fn fetch_error<E>(url: &str, error: E) -> ToolExecutionError
where
    E: Error + Send + Sync + 'static,
{
    ToolExecutionError::network(format!(
        "Cannot fetch URL \"{url}\": the network request failed."
    ))
    .with_code(error_codes::NETWORK_ERROR)
    .with_source(error)
}

pub(super) fn response_body_error<E>(url: &str, error: E) -> ToolExecutionError
where
    E: Error + Send + Sync + 'static,
{
    ToolExecutionError::network(format!(
        "Cannot read the response body for URL \"{url}\": the network request failed."
    ))
    .with_code(error_codes::NETWORK_ERROR)
    .with_source(error)
}

pub(super) fn timeout_error(url: &str) -> ToolExecutionError {
    ToolExecutionError::timeout(format!(
        "Fetching URL \"{url}\" exceeded the server timeout."
    ))
    .with_code(error_codes::TIMEOUT)
}
