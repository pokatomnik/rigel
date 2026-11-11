use std::{sync::Arc, time::Duration};

use reqwest::{Client, Url};

const GLOBAL_TOOL_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REDIRECTS: usize = 10;

/// Builds the shared HTTP client with redirect checks delegated to the supplied URL policy.
pub(crate) async fn build_client(target_allowed: fn(&Url) -> bool) -> anyhow::Result<Arc<Client>> {
    let config = Client::builder()
        .no_proxy()
        .gzip(true)
        .brotli(true)
        .connect_timeout(GLOBAL_TOOL_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= MAX_REDIRECTS || !target_allowed(attempt.url()) {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .user_agent(include_str!("../../user_agents.txt"))
        .build()?;
    Ok(Arc::new(config))
}
