use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Configuration for an HTTP MCP server before a transport adapter connects it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct McpHttpConfig {
    /// Optional endpoint; validation belongs to the MCP adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
}

/// Configuration for a child-process MCP server before a transport adapter connects it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct McpStdioConfig {
    /// Executable name or path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<String>,
    /// Positional command arguments.
    #[serde(default)]
    pub(crate) args: Vec<String>,
    /// Environment overrides for the child process.
    #[serde(default)]
    pub(crate) env: HashMap<String, String>,
}

/// Data-only MCP server configuration selected by name in a Rigel profile.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub(crate) enum McpServerConfig {
    /// A stdio child process.
    #[serde(rename = "stdio")]
    Stdio(McpStdioConfig),
    /// A streamable HTTP endpoint.
    #[serde(rename = "http")]
    Http(McpHttpConfig),
}

impl McpServerConfig {
    /// Merges matching server variants while replacing mismatched project definitions.
    pub(crate) fn merge(global: Self, project: Self) -> Self {
        match (global, project) {
            (Self::Stdio(global), Self::Stdio(project)) => Self::Stdio(McpStdioConfig {
                command: project.command.or(global.command),
                args: merge_vec(global.args, project.args),
                env: merge_map(global.env, project.env),
            }),
            (Self::Http(global), Self::Http(project)) => Self::Http(McpHttpConfig {
                url: project.url.or(global.url),
            }),
            (_, project) => project,
        }
    }
}

fn merge_vec<T>(mut global: Vec<T>, project: Vec<T>) -> Vec<T> {
    global.extend(project);
    global
}

fn merge_map(
    mut global: HashMap<String, String>,
    project: HashMap<String, String>,
) -> HashMap<String, String> {
    global.extend(project);
    global
}
