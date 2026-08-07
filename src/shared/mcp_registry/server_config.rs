use serde::{Deserialize, Serialize};

use super::http_config::HttpConfig;
use super::stdio_config::StdioConfig;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum ServerConfig {
    #[serde(rename = "stdio")]
    Stdio(StdioConfig),

    #[serde(rename = "http")]
    Http(HttpConfig),
}
