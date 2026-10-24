//! `mcp.json`: what its servers are, and turning one on or off. Plain
//! functions from text to text; the caller reads and writes the file.

use protocol::{McpKind, McpServer};
use serde_json::Value;

use crate::error::Error;
use crate::json;

const FILE: &str = "mcp.json";

pub fn servers(text: &str) -> Result<Vec<McpServer>, Error> {
    let doc = json::object(text, FILE)?;
    let Some(entries) = doc.get("mcpServers") else {
        return Ok(Vec::new());
    };
    let entries = entries
        .as_object()
        .ok_or_else(|| Error::invalid(FILE, "mcpServers isn't an object"))?;
    entries
        .iter()
        .map(|(name, entry)| server(name, entry))
        .collect()
}

fn server(name: &str, entry: &Value) -> Result<McpServer, Error> {
    let entry = entry
        .as_object()
        .ok_or_else(|| Error::invalid(FILE, format!("{name} isn't an object")))?;
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let (kind, runs) = match (text("command"), text("url")) {
        (Some(command), _) => {
            let args = entry
                .get("args")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str);
            (
                McpKind::Local,
                std::iter::once(command.as_str())
                    .chain(args)
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }
        (None, Some(url)) => (McpKind::Remote, url),
        (None, None) => {
            return Err(Error::invalid(
                FILE,
                format!("{name} has neither a command nor a url"),
            ));
        }
    };
    let env = entry
        .get("env")
        .and_then(Value::as_object)
        .map(|env| {
            env.iter()
                .map(|(key, value)| format!("{key}={}", value.as_str().unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    Ok(McpServer {
        name: name.to_string(),
        kind,
        runs,
        enabled: entry
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        description: text("description"),
        env,
    })
}

/// The file with one server turned on or off. Turning one on drops its
/// `enabled` key, as Pi treats a server without one as on.
pub fn set_enabled(text: &str, name: &str, enabled: bool) -> Result<String, Error> {
    let mut doc = json::object(text, FILE)?;
    let entry = doc
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .and_then(|entries| entries.get_mut(name))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error::NotFound(format!("no MCP server called {name}")))?;
    if enabled {
        entry.remove("enabled");
    } else {
        entry.insert("enabled".into(), Value::Bool(false));
    }
    Ok(json::pretty(&doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
  "mcpServers": {
    "grafana": {
      "command": "mcp-grafana",
      "env": { "GRAFANA_URL": "https://grafana.shop.internal", "GRAFANA_TOKEN": "${GRAFANA_TOKEN}" }
    },
    "azure-devops": { "command": "npx", "args": ["-y", "@azure-devops/mcp", "shop"], "enabled": false },
    "sentry": { "url": "https://mcp.sentry.dev/mcp", "description": "Errors" }
  }
}"#;

    #[test]
    fn reads_local_and_remote_servers() {
        let servers = servers(SAMPLE).unwrap();
        assert_eq!(servers.len(), 3);
        assert_eq!(servers[0].name, "grafana");
        assert_eq!(servers[0].kind, McpKind::Local);
        assert_eq!(
            servers[0].env,
            [
                "GRAFANA_URL=https://grafana.shop.internal",
                "GRAFANA_TOKEN=${GRAFANA_TOKEN}"
            ]
        );
        assert_eq!(servers[1].runs, "npx -y @azure-devops/mcp shop");
        assert!(!servers[1].enabled);
        assert_eq!(servers[2].kind, McpKind::Remote);
        assert_eq!(servers[2].description.as_deref(), Some("Errors"));
    }

    #[test]
    fn no_file_means_no_servers() {
        assert!(servers("").unwrap().is_empty());
        assert!(servers("{}").unwrap().is_empty());
    }

    #[test]
    fn turns_servers_off_and_on() {
        let off = set_enabled(SAMPLE, "grafana", false).unwrap();
        assert!(!servers(&off).unwrap()[0].enabled);
        let on = set_enabled(&off, "grafana", true).unwrap();
        assert!(servers(&on).unwrap()[0].enabled);
        assert!(!on.contains(r#""enabled": true"#));
    }

    #[test]
    fn keeps_the_order_servers_are_written_in() {
        let off = set_enabled(SAMPLE, "sentry", false).unwrap();
        let names: Vec<_> = servers(&off).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["grafana", "azure-devops", "sentry"]);
    }

    #[test]
    fn an_unknown_server_is_not_found() {
        assert!(matches!(
            set_enabled(SAMPLE, "nope", false),
            Err(Error::NotFound(_))
        ));
    }
}
