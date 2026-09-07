use std::collections::HashMap;

use super::rigel_config::RigelConfig;
use crate::shared::mcp_registry::{
    http_config::HttpConfig, server_config::ServerConfig, stdio_config::StdioConfig,
};

impl RigelConfig {
    pub fn mcp_servers(&self) -> &HashMap<String, ServerConfig> {
        self.servers_ref()
    }

    pub(super) fn merge_servers(
        mut global: HashMap<String, ServerConfig>,
        project: HashMap<String, ServerConfig>,
    ) -> HashMap<String, ServerConfig> {
        for (name, project_server) in project {
            let merged = match global.remove(&name) {
                Some(global_server) => Self::merge_server(global_server, project_server),
                None => project_server,
            };
            global.insert(name, merged);
        }
        global
    }

    fn merge_server(global: ServerConfig, project: ServerConfig) -> ServerConfig {
        match (global, project) {
            (ServerConfig::Stdio(global), ServerConfig::Stdio(project)) => {
                ServerConfig::Stdio(StdioConfig {
                    command: project.command.or(global.command),
                    args: Self::merge_vec(global.args, project.args),
                    env: Self::merge_string_map(global.env, project.env),
                })
            }
            (ServerConfig::Http(global), ServerConfig::Http(project)) => {
                ServerConfig::Http(HttpConfig {
                    url: project.url.or(global.url),
                })
            }
            (_, project) => project,
        }
    }

    fn merge_vec<T>(mut global: Vec<T>, project: Vec<T>) -> Vec<T> {
        global.extend(project);
        global
    }

    fn merge_string_map(
        mut global: HashMap<String, String>,
        project: HashMap<String, String>,
    ) -> HashMap<String, String> {
        global.extend(project);
        global
    }
}
