//! The MCP servers a run gets beside Priory's own: those its agent names, found by name in the
//! person's own harness config when the run starts. A definition can hold tokens, so no error here
//! quotes a config's text.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

/// The servers of the person's config that a run starts with, or must not start with.
#[derive(Debug, Default, PartialEq)]
pub struct Servers {
    /// Definitions copied from the person's Claude Code config, by name, for `--mcp-config`.
    pub defined: Map<String, Value>,
    /// Servers of the person's Codex config that the run turns off, since its agent names none of them.
    pub off: Vec<String>,
    /// The claude.ai connectors a Claude Code run gets, by Claude Code's server name for each,
    /// `claude.ai <name>`. A run with one starts without `--strict-mcp-config`, since Claude Code
    /// loads no connector under it.
    pub connectors: Vec<String>,
    /// For a run with connectors: a permission rule `mcp__<server>` for each other server of the
    /// person's Claude Code config, which that run would load too.
    pub denied: Vec<String>,
}

/// What Claude Code puts before a claude.ai connector's name to make its server's name.
pub const CONNECTOR: &str = "claude.ai ";

/// Claude Code's server name for the claude.ai connector `name`: `claude.ai Linear` for `Linear`.
pub fn connector_server(name: &str) -> String {
    format!("{CONNECTOR}{name}")
}

/// The server part of a tool's name, `mcp__<it>__<tool>`, as Claude Code 2.1.330 writes it: each
/// UTF-16 unit outside `[A-Za-z0-9_-]` becomes `_`, and for a connector, runs of `_` become one,
/// with none at either end. So `claude.ai Google Drive` gives `claude_ai_Google_Drive`.
pub fn tool_server(server: &str) -> String {
    let mut name = String::new();
    for c in server.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            name.push(c);
        } else {
            name.extend(std::iter::repeat_n('_', c.len_utf16()));
        }
    }
    if !server.starts_with(CONNECTOR) {
        return name;
    }
    let mut collapsed = String::new();
    for c in name.chars() {
        if !(c == '_' && collapsed.ends_with('_')) {
            collapsed.push(c);
        }
    }
    collapsed.trim_matches('_').to_owned()
}

/// Where Claude Code reads the MCP servers an administrator manages for every user of the machine.
fn managed_config() -> PathBuf {
    if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode/managed-mcp.json")
    } else {
        PathBuf::from("/etc/claude-code/managed-mcp.json")
    }
}

/// The name Priory's own server takes in every run.
const PRIORY: &str = "priory";

/// The person's Claude Code config: `$CLAUDE_CONFIG_DIR/.claude.json`, else `~/.claude.json`.
pub fn claude_config() -> PathBuf {
    match std::env::var_os("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir).join(".claude.json"),
        None => home().join(".claude.json"),
    }
}

/// The person's Codex config: `$CODEX_HOME/config.toml`, else `~/.codex/config.toml`.
pub fn codex_config() -> PathBuf {
    match std::env::var_os("CODEX_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("config.toml"),
        None => home().join(".codex/config.toml"),
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from)
}

/// Copies each of `names` from the user-scope `mcpServers` of the person's Claude Code config.
/// With `connectors`, a run in the worktree `dir` also gets a rule that denies each other server of
/// that config, of `dir`'s `.mcp.json` and of the machine's managed config.
pub fn claude(names: &[String], connectors: &[String], dir: &Path) -> Result<Servers, String> {
    let path = claude_config();
    let text = read(&path)?;
    let mut servers = claude_from(&text, &path, names)?;
    if connectors.is_empty() {
        return Ok(servers);
    }
    let mut configs = vec![parse(&text, &path)?];
    for path in [dir.join(".mcp.json"), managed_config()] {
        configs.push(parse(&read(&path)?, &path)?);
    }
    servers.connectors = connectors
        .iter()
        .map(|name| connector_server(name))
        .collect();
    servers.denied = denials(&configs, names, &servers.connectors);
    Ok(servers)
}

/// Every server name a Claude Code config holds: its `mcpServers`, each project's `mcpServers`
/// (the local scope), and `claudeAiMcpEverConnected`, the connectors that once connected here.
fn configured(config: &Value) -> Vec<String> {
    let keys = |servers: Option<&Value>| -> Vec<String> {
        servers
            .and_then(Value::as_object)
            .map(|servers| servers.keys().cloned().collect())
            .unwrap_or_default()
    };
    let mut names = keys(config.get("mcpServers"));
    if let Some(projects) = config.get("projects").and_then(Value::as_object) {
        for project in projects.values() {
            names.extend(keys(project.get("mcpServers")));
        }
    }
    if let Some(connected) = config
        .get("claudeAiMcpEverConnected")
        .and_then(Value::as_array)
    {
        names.extend(
            connected
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
    names
}

/// A deny rule for each server of `configs` that is neither Priory's, one of `names` nor one of
/// `connectors`, compared by the server part of their tools' names, sorted.
fn denials(configs: &[Value], names: &[String], connectors: &[String]) -> Vec<String> {
    let kept: Vec<String> = std::iter::once(PRIORY)
        .chain(names.iter().map(String::as_str))
        .chain(connectors.iter().map(String::as_str))
        .map(tool_server)
        .collect();
    let mut denied: Vec<String> = configs
        .iter()
        .flat_map(configured)
        .map(|server| tool_server(&server))
        .filter(|server| !server.is_empty() && !kept.contains(server))
        .map(|server| format!("mcp__{server}"))
        .collect();
    denied.sort();
    denied.dedup();
    denied
}

/// Turns off every server of the person's Codex config, and of the worktree `dir`'s project
/// config, that `names` leaves out, and refuses a name neither defines.
pub fn codex(names: &[String], dir: &Path) -> Result<Servers, String> {
    let user = codex_config();
    let project = dir.join(".codex/config.toml");
    let mut defined = codex_names(&read(&user)?, &user)?;
    for name in codex_names(&read(&project)?, &project)? {
        if !defined.contains(&name) {
            defined.push(name);
        }
    }
    reserved(names)?;
    if let Some(name) = names.iter().find(|name| !defined.contains(name)) {
        return Err(format!(
            "names MCP server {name}, which neither {} nor {} defines under mcp_servers",
            user.display(),
            project.display()
        ));
    }
    let mut off: Vec<String> = defined
        .into_iter()
        .filter(|name| name != PRIORY && !names.contains(name))
        .collect();
    off.sort();
    Ok(Servers {
        off,
        ..Servers::default()
    })
}

/// A missing config defines no server.
fn read(path: &Path) -> Result<String, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!(
            "cannot read its MCP servers: {}: {error}",
            path.display()
        )),
    }
}

fn reserved(names: &[String]) -> Result<(), String> {
    if names.iter().any(|name| name == PRIORY) {
        return Err(format!(
            "names MCP server {PRIORY}, which Priory serves to every run itself; remove it from mcpServers"
        ));
    }
    Ok(())
}

/// A JSON config's text, or null for an empty one.
fn parse(text: &str, path: &Path) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(text).map_err(|error| {
        // serde_json's message can quote the text; its position cannot.
        format!(
            "cannot read its MCP servers: {} is not valid JSON at line {} column {}",
            path.display(),
            error.line(),
            error.column()
        )
    })
}

fn claude_from(text: &str, path: &Path, names: &[String]) -> Result<Servers, String> {
    reserved(names)?;
    let config = parse(text, path)?;
    let mut defined = Map::new();
    for name in names {
        let Some(server) = config
            .get("mcpServers")
            .and_then(|servers| servers.get(name))
            .filter(|server| server.is_object())
        else {
            return Err(format!(
                "names MCP server {name}, which the mcpServers of {} do not define; add it with claude mcp add --scope user",
                path.display()
            ));
        };
        defined.insert(name.clone(), server.clone());
    }
    Ok(Servers {
        defined,
        ..Servers::default()
    })
}

/// The names under `mcp_servers` in a Codex config's text.
fn codex_names(text: &str, path: &Path) -> Result<Vec<String>, String> {
    let config: toml::Table = toml::from_str(text).map_err(|error| {
        // toml's message quotes the line it failed on, which can hold a token.
        let line = error.span().map_or(0, |span| {
            let before = &text.as_bytes()[..span.start.min(text.len())];
            before.iter().filter(|byte| **byte == b'\n').count() + 1
        });
        format!(
            "cannot read its MCP servers: {} is not valid TOML at line {line}",
            path.display()
        )
    })?;
    Ok(match config.get("mcp_servers") {
        Some(toml::Value::Table(servers)) => servers
            .iter()
            .filter(|(name, server)| server.is_table() && overridable(name))
            .map(|(name, _)| name.clone())
            .collect(),
        _ => Vec::new(),
    })
}

/// A name Codex's `-c mcp_servers.<name>.enabled=false` can reach; Codex refuses any other.
fn overridable(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn claude_copies_the_named_servers_and_refuses_an_unknown_one_without_quoting_the_config() {
        let text = r#"{"mcpServers": {"sentry": {"command": "npx", "env": {"T": "s3cret"}}, "linear": {"url": "u"}}}"#;
        let path = Path::new("/c/.claude.json");
        let found = claude_from(text, path, &names(&["sentry"])).unwrap();
        assert_eq!(found.defined.keys().collect::<Vec<_>>(), ["sentry"]);
        assert_eq!(found.defined["sentry"]["env"]["T"], "s3cret");
        assert!(claude_from(text, path, &[]).unwrap().defined.is_empty());
        let refused = claude_from(text, path, &names(&["honeycomb"])).unwrap_err();
        assert!(
            refused.starts_with("names MCP server honeycomb, which"),
            "{refused}"
        );
        let broken = claude_from(r#"{"mcpServers": {"s3cret"#, path, &names(&["a"])).unwrap_err();
        assert!(!broken.contains("s3cret"), "{broken}");
        assert!(
            claude_from("", path, &names(&["priory"]))
                .unwrap_err()
                .contains("Priory serves")
        );
    }

    #[test]
    fn a_broken_codex_config_is_refused_by_line_without_quoting_it() {
        let text = "model = \"x\"\n[mcp_servers.a]\nenv = { T = \"s3cret\" \n";
        let refused = codex_names(text, Path::new("/c/config.toml")).unwrap_err();
        assert!(!refused.contains("s3cret"), "{refused}");
        assert!(
            refused.starts_with(
                "cannot read its MCP servers: /c/config.toml is not valid TOML at line"
            )
        );
    }

    #[test]
    fn a_tool_server_is_the_name_claude_code_gives_a_servers_tools() {
        assert_eq!(tool_server("claude.ai Linear"), "claude_ai_Linear");
        assert_eq!(
            tool_server("claude.ai Google Drive"),
            "claude_ai_Google_Drive"
        );
        assert_eq!(tool_server("claude.ai  A -- B. "), "claude_ai_A_--_B");
        assert_eq!(tool_server("my.server"), "my_server");
        assert_eq!(tool_server("a..b"), "a__b");
        assert_eq!(tool_server("caf\u{e9}"), "caf_");
        assert_eq!(tool_server("x\u{1f600}"), "x__");
    }

    #[test]
    fn a_run_with_connectors_denies_every_other_server_of_every_scope() {
        let user = serde_json::json!({
            "mcpServers": {"sentry": {}, "linear": {}, "priory": {}},
            "projects": {"/p": {"mcpServers": {"notion": {}, "my.db": {}}}},
            "claudeAiMcpEverConnected": ["claude.ai Slack", "claude.ai Linear", 7],
        });
        let project = serde_json::json!({"mcpServers": {"figma": {}, "sentry": {}}});
        let managed = serde_json::json!({"mcpServers": {"corp": {}}});
        let denied = denials(
            &[user, project, managed, Value::Null],
            &names(&["sentry"]),
            &[connector_server("Linear")],
        );
        assert_eq!(
            denied,
            [
                "mcp__claude_ai_Slack",
                "mcp__corp",
                "mcp__figma",
                "mcp__linear",
                "mcp__my_db",
                "mcp__notion"
            ]
        );
    }

    proptest! {
        #[test]
        fn the_denials_cover_every_server_the_run_does_not_name_and_none_it_does(
            user in proptest::collection::btree_set("[a-zA-Z0-9 ._-]{1,12}", 0..6),
            local in proptest::collection::btree_set("[a-zA-Z0-9 ._-]{1,12}", 0..6),
            project in proptest::collection::btree_set("[a-zA-Z0-9 ._-]{1,12}", 0..6),
            managed in proptest::collection::btree_set("[a-zA-Z0-9 ._-]{1,12}", 0..4),
            connected in proptest::collection::btree_set("[A-Za-z][A-Za-z0-9 ._-]{0,12}", 0..6),
            picks in proptest::collection::vec(any::<bool>(), 32),
        ) {
            let object = |names: &std::collections::BTreeSet<String>| -> Value {
                names.iter().map(|name| (name.clone(), serde_json::json!({}))).collect::<Map<_, _>>().into()
            };
            let connected: Vec<String> = connected.iter().map(|name| connector_server(name)).collect();
            let configs = [
                serde_json::json!({"mcpServers": object(&user), "projects": {"/p": {"mcpServers": object(&local)}},
                    "claudeAiMcpEverConnected": connected}),
                serde_json::json!({"mcpServers": object(&project)}),
                serde_json::json!({"mcpServers": object(&managed)}),
            ];
            let every: Vec<String> = configs.iter().flat_map(configured).collect();
            let mut pick = picks.iter().cycle();
            let named: Vec<String> = user.iter().filter(|_| *pick.next().unwrap()).cloned().collect();
            let connectors: Vec<String> = connected.iter().filter(|_| *pick.next().unwrap()).cloned().collect();
            let denied = denials(&configs, &named, &connectors);
            let kept: Vec<String> = std::iter::once(PRIORY.to_owned())
                .chain(named.iter().cloned())
                .chain(connectors.iter().cloned())
                .map(|server| tool_server(&server))
                .collect();
            for server in &every {
                let rule = format!("mcp__{}", tool_server(server));
                if kept.contains(&tool_server(server)) {
                    prop_assert!(!denied.contains(&rule), "{rule} denies a server the run names");
                } else {
                    prop_assert!(denied.contains(&rule), "{server} is not denied: {denied:?}");
                }
            }
        }

        #[test]
        fn the_config_readers_return_or_refuse_without_panicking(
            text in "\\PC{0,200}",
            mutated in proptest::collection::vec(any::<u8>(), 0..200),
        ) {
            let path = Path::new("/c/x");
            let wanted = names(&["sentry"]);
            let _ = claude_from(&text, path, &wanted);
            let _ = codex_names(&text, path);
            let mutated = String::from_utf8_lossy(&mutated);
            let _ = claude_from(&mutated, path, &wanted);
            let _ = codex_names(&mutated, path);
            let seeded = format!("[mcp_servers.{text}]\ncommand = \"x\"");
            let _ = codex_names(&seeded, path);
        }
    }
}
