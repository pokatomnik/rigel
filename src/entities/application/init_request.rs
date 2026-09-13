/// Contains the user choices needed to initialize a configuration file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InitRequest {
    base_url: String,
    api_key_env: Option<String>,
}

impl InitRequest {
    /// Creates an initialization request with an API endpoint and optional key variable.
    pub(crate) fn new(base_url: String, api_key_env: Option<String>) -> Self {
        Self {
            base_url,
            api_key_env,
        }
    }

    /// Returns the configured OpenAI-compatible API base URL.
    pub(crate) fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    /// Returns the environment variable name containing the API key, when configured.
    pub(crate) fn api_key_env(&self) -> Option<&str> {
        self.api_key_env.as_deref()
    }
}
