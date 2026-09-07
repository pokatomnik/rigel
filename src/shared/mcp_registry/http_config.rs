use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub(crate) struct HttpConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
}
