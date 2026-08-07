use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
pub(crate) struct HttpConfig {
    pub(crate) url: String,
}
