#[allow(dead_code)]
mod harness;

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

const WORKFLOW: &str = r#"
start = "implement"

[[steps]]
id = "implement"
decides = "model"
agent = "implementer"
prompt = "Implement the issue."
effect = "reversible"
next = { passed = "done", failed = "stop" }
"#;

const AGENT: &str = "+++\ndescription = \"Implements one task.\"\nharness = \"claude-code\"\n+++\n";

/// A factory on a bus and a Priory home under the test's own directory. Dropping it stops the
/// gateway it started.
struct Setup {
    dir: PathBuf,
    gateway: Option<Child>,
}

impl Setup {
    fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("gateway")
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        let files = [
            ("factory/factory.toml", "name = \"demo\"\n"),
            ("factory/agents/implementer/agent.md", AGENT),
            ("factory/workflows/implement.toml", WORKFLOW),
        ];
        for (path, content) in files {
            let file = dir.join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        Setup { dir, gateway: None }
    }

    fn command(&self, now: &str) -> Command {
        let mut command = harness::priory();
        command
            .env("HERDR_BUS_DB", self.dir.join("bus.db"))
            .env("PRIORY_DB", self.dir.join("priory.db"))
            .env("PRIORY_HOME", self.dir.join("home"))
            .env("PRIORY_NOW", now);
        command
    }

    fn factory(&self) -> String {
        self.dir.join("factory").to_str().unwrap().to_owned()
    }

    /// Makes a grant at `now` that lasts `span`, and returns its id and token.
    fn grant(&self, now: &str, span: &str) -> (String, String) {
        let output = self
            .command(now)
            .args(["gateway", "grant", &self.factory(), "--for", span])
            .output()
            .unwrap();
        let made: Value = serde_json::from_slice(&succeeded(&output).stdout).unwrap();
        (
            made["grant"].as_str().unwrap().to_owned(),
            made["token"].as_str().unwrap().to_owned(),
        )
    }

    fn revoke(&self, now: &str, grant: &str) {
        let output = self
            .command(now)
            .args(["gateway", "revoke", &self.factory(), grant])
            .output()
            .unwrap();
        succeeded(&output);
    }

    /// Starts the gateway with its clock at `now` and returns the address it listens on.
    fn start(&mut self, now: &str) -> String {
        let mut child = self
            .command(now)
            .args(["gateway", &self.factory(), "--listen", "127.0.0.1:0"])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        self.gateway = Some(child);
        let mut said = String::new();
        loop {
            let mut line = String::new();
            assert!(
                stderr.read_line(&mut line).unwrap() > 0,
                "the gateway exited before it listened: {said}"
            );
            if let Some(address) = line.trim().strip_prefix("priory: serving on http://") {
                std::thread::spawn(move || std::io::copy(&mut stderr, &mut std::io::sink()));
                return address.to_owned();
            }
            said.push_str(&line);
        }
    }

    fn audit(&self) -> Vec<Value> {
        fs::read_to_string(self.dir.join("home/diocese-gateway.audit.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

impl Drop for Setup {
    fn drop(&mut self) {
        if let Some(mut gateway) = self.gateway.take() {
            let _ = gateway.kill();
            let _ = gateway.wait();
        }
    }
}

fn succeeded(output: &Output) -> &Output {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Posts one JSON-RPC message to `/mcp` and returns the status and the body.
fn call(address: &str, token: Option<&str>, message: &Value) -> (u16, String) {
    let body = message.to_string();
    let auth = token.map_or(String::new(), |token| {
        format!("Authorization: Bearer {token}\r\n")
    });
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let status = response[9..12].parse().unwrap();
    let body = response.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    (status, body.to_owned())
}

fn whoami() -> Value {
    json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "whoami", "arguments": {} } })
}

#[test]
fn gateway_serves_whoami_to_a_live_grant_and_refuses_and_audits_every_other_token() {
    let mut setup = Setup::new("refuses");
    let (live, token) = setup.grant("2026-11-26T12:00:00Z", "1h");
    let (revoked, revoked_token) = setup.grant("2026-11-26T12:00:00Z", "1h");
    setup.revoke("2026-11-26T12:00:00Z", &revoked);
    let (_, expired_token) = setup.grant("2026-11-26T10:00:00Z", "1m");
    let address = setup.start("2026-11-26T12:00:30Z");

    let initialize = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-11-25" } });
    let (status, body) = call(&address, Some(&token), &initialize);
    assert_eq!(status, 200, "{body}");
    let reply: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(reply["result"]["protocolVersion"], "2025-11-25");

    let list = json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" });
    let (_, body) = call(&address, Some(&token), &list);
    let tools: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(tools["result"]["tools"][0]["name"], "whoami");

    let (status, body) = call(&address, Some(&token), &whoami());
    assert_eq!(status, 200, "{body}");
    let text: Value = serde_json::from_str(&body).unwrap();
    let text = text["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains(&live), "{text}");
    assert!(text.contains("2026-11-26T13:00:00Z"), "{text}");

    for (token, why) in [
        (None, "Authorization: Bearer"),
        (Some("0123456789abcdef0123456789abcdef"), "no run grant"),
        (Some(expired_token.as_str()), "has expired"),
        (Some(revoked_token.as_str()), "is revoked"),
    ] {
        let (status, body) = call(&address, token, &whoami());
        assert_eq!(status, 401, "{why}: {body}");
        assert!(body.contains(why), "{why}: {body}");
    }

    let audit = setup.audit();
    let refused: Vec<_> = audit.iter().filter(|line| line["status"] == 401).collect();
    assert_eq!(refused.len(), 4, "{audit:#?}");
    assert!(
        audit
            .iter()
            .any(|line| line["grant"] == live.as_str() && line["tool"] == "whoami")
    );
    let written = fs::read_to_string(setup.dir.join("home/diocese-gateway.audit.jsonl")).unwrap();
    for secret in [&token, &revoked_token, &expired_token] {
        assert!(!written.contains(secret.as_str()));
    }
}

#[test]
fn gateway_answers_429_past_60_calls_of_a_grant_and_10_refusals_a_minute() {
    let mut setup = Setup::new("limits");
    let (_, token) = setup.grant("2026-11-26T12:00:00Z", "1h");
    let address = setup.start("2026-11-26T12:00:30Z");
    let ping = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });

    for n in 1..=60 {
        let (status, body) = call(&address, Some(&token), &ping);
        assert_eq!(status, 200, "call {n}: {body}");
    }
    let (status, body) = call(&address, Some(&token), &ping);
    assert_eq!(status, 429, "{body}");

    for n in 1..=10 {
        let (status, body) = call(&address, Some("wrong"), &ping);
        assert_eq!(status, 401, "refusal {n}: {body}");
    }
    let (status, body) = call(&address, Some("wrong"), &ping);
    assert_eq!(status, 429, "{body}");
}

#[test]
fn gateway_refuses_to_listen_beyond_loopback() {
    let setup = Setup::new("loopback");
    let output = setup
        .command("2026-11-26T12:00:00Z")
        .args(["gateway", &setup.factory(), "--listen", "0.0.0.0:0"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("beyond this machine"), "{stderr}");
}
