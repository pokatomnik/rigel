use rig::providers::openai;

use super::{agent::Agent, agent_config::AgentConfig, dependencies::AgentDependencies};

impl Agent {
    pub(super) fn build_client(
        config: &AgentConfig,
        deps: &AgentDependencies,
    ) -> anyhow::Result<openai::Client> {
        let api_key = config.api_key.clone().unwrap_or_default();
        Ok(openai::Client::builder()
            .api_key(api_key)
            .base_url(config.base_url.clone())
            .http_client(deps.http_client.as_ref().clone())
            .build()?)
    }
}
