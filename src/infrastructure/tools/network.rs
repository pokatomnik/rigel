use std::future::Future;
use std::pin::Pin;

use crate::entities::tool::tool_entities::{HttpFetchRequest, HttpFetchResponse};

/// Fetches only targets accepted by the SSRF/redirect policy.
pub(crate) trait SafeHttpFetcher: Send + Sync {
    /// Validates the initial and every redirected target before reading a body.
    fn fetch(
        &self,
        request: HttpFetchRequest,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<HttpFetchResponse>> + Send + '_>>;
}
