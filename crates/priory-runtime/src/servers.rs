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
pub fn claude(names: &[String]) -> Result<Servers, String> {
    let path = claude_config();
    let text = read(&path)?;
    claude_from(&text, &path, names)
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
        defined: Map::new(),
        off,
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

fn claude_from(text: &str, path: &Path, names: &[String]) -> Result<Servers, String> {
    reserved(names)?;
    let config: Value = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(text).map_err(|error| {
            // serde_json's message can quote the text; its position cannot.
            format!(
                "cannot read its MCP servers: {} is not valid JSON at line {} column {}",
                path.display(),
                error.line(),
                error.column()
            )
        })?
    };
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
        off: Vec::new(),
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

    proptest! {
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
