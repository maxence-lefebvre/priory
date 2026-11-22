mod conform;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn reference() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference")
}

fn examples() -> PathBuf {
    reference().join("examples")
}

/// Every file under `dir`, as a path relative to `root`.
fn files(root: &Path, dir: &Path) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(files(root, &path));
        } else {
            let relative = path.strip_prefix(root).unwrap();
            found.insert(relative.to_string_lossy().into_owned());
        }
    }
    found
}

/// Copies the example factory to `to`, with the `deliver` script its `factory.toml` names, which
/// the reference pages do not show.
fn example_factory(to: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::remove_dir_all(to);
    copy(&examples().join("factory"), to);
    let deliver = to.join("bin/deliver");
    fs::create_dir_all(deliver.parent().unwrap()).unwrap();
    fs::write(&deliver, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&deliver, fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn the_example_factory_passes_check() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reference-example");
    example_factory(&dir);
    let output = Command::new(env!("CARGO_BIN_EXE_priory"))
        .arg("check")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn every_starter_factory_passes_check_and_its_explanation() {
    let starters = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut checked = 0;
    for entry in fs::read_dir(&starters).unwrap() {
        let starter = entry.unwrap().path();
        if !starter.is_dir() {
            continue;
        }
        for flags in [&[][..], &["--explain"]] {
            let output = Command::new(env!("CARGO_BIN_EXE_priory"))
                .arg("check")
                .args(flags)
                .arg(&starter)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{} {flags:?}: {}{}",
                starter.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        checked += 1;
    }
    assert!(checked > 0, "no starter under {}", starters.display());
}

#[test]
fn the_batches_starter_passes_check() {
    let starter = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/batches");

    let output = Command::new(env!("CARGO_BIN_EXE_priory"))
        .arg("check")
        .arg(&starter)
        .output()
        .unwrap();

    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "ok: 2 agents, 1 trigger, 2 workflows, 1 runner\n"
    );
}

/// Values whose shape a config schema checks with a pattern, valid or not to the loader.
#[test]
fn the_config_schemas_accept_the_values_the_loader_accepts() {
    let schedule = "source = \"schedule\"\nrepo = \"owner/name\"\nworkflow = \"implement\"\n";
    let cases = [
        ("at", "09:00"),
        ("at", "9:00"),
        ("at", "23:59"),
        ("at", "24:00"),
        ("at", "29:00"),
        ("at", "09:00:00"),
        ("every", "90s"),
        ("every", "2h"),
        ("every", "0m"),
        ("every", "1.5h"),
        ("every", "60"),
        ("until", "18:00"),
        ("until", "9h"),
        ("until", "6pm"),
    ]
    .map(|(key, value)| {
        let at = if key == "at" { "" } else { "at = \"08:00\"\n" };
        let text = format!("{schedule}{at}{key} = \"{value}\"\n");
        ("triggers/edge.toml", "trigger", text)
    });
    let shares = ["60%", "100%", "0.5%", "0%", "101%", "150%", "60", "sixty%"].map(|value| {
        let text = format!("platform = {{ os = \"linux\" }}\nagents = {{ cpu = \"{value}\" }}\n");
        ("runners/edge.toml", "runner", text)
    });
    let sizes = ["6GB", "1.5TB", ".5GB", "512MB", "0GB", "6G", "6 GB"].map(|value| {
        let text =
            format!("+++\ndescription = \"d\"\nharness = \"codex\"\nmemory = \"{value}\"\n+++\n");
        ("agents/edge/agent.md", "agent", text)
    });
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("reference-edge");
    let mut disagree = Vec::new();
    for (file, schema, text) in cases.into_iter().chain(shares).chain(sizes) {
        example_factory(&dir);
        fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
        fs::write(dir.join(file), &text).unwrap();
        let loads = Command::new(env!("CARGO_BIN_EXE_priory"))
            .arg("check")
            .arg(&dir)
            .output()
            .unwrap()
            .status
            .success();
        let value = match file.ends_with(".md") {
            true => frontmatter(&text),
            false => toml::from_str(&text).unwrap(),
        };
        let validates = conform::errors(&format!("config/{schema}.json"), &value).is_empty();
        if loads != validates {
            disagree.push(format!(
                "{file}: {text:?} loads {loads}, validates {validates}"
            ));
        }
    }
    assert!(disagree.is_empty(), "{disagree:#?}");
}

fn copy(from: &Path, to: &Path) {
    for file in files(from, from) {
        let target = to.join(&file);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(from.join(&file), target).unwrap();
    }
}

#[test]
fn every_example_config_matches_its_schema() {
    let dir = examples().join("factory");
    for file in files(&dir, &dir) {
        let text = fs::read_to_string(dir.join(&file)).unwrap();
        let schema = match file.split('/').next().unwrap() {
            "factory.toml" => "factory",
            "agents" => "agent",
            "triggers" => "trigger",
            "workflows" => "workflow",
            "runners" => "runner",
            "views" => "view",
            "scorers" if file.ends_with(".eval.toml") => "judge-cases",
            "scorers" if file.ends_with(".md") => continue,
            "scorers" => "scorer",
            "benchmarks" => "benchmark",
            "memories" => "memory",
            "scripts" => continue,
            other => panic!("no schema for {other}"),
        };
        let value: serde_json::Value = match file.rsplit_once('.').unwrap().1 {
            "toml" => toml::from_str(&text).unwrap(),
            "yaml" => serde_saphyr::from_str(&text).unwrap(),
            _ => frontmatter(&text),
        };
        conform::matches(&format!("config/{schema}.json"), &value);
    }
}

/// The TOML or YAML between an agent file's `+++` or `---` fences.
fn frontmatter(text: &str) -> serde_json::Value {
    let fence = &text[..3];
    let (inside, _) = text[4..].split_once(&format!("\n{fence}\n")).unwrap();
    match fence {
        "+++" => toml::from_str(inside).unwrap(),
        _ => serde_saphyr::from_str(inside).unwrap(),
    }
}

#[test]
fn every_event_type_has_an_example_that_matches_its_schema() {
    let dir = examples().join("events");
    let mut kinds = BTreeSet::new();
    for file in files(&dir, &dir) {
        let message: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.join(&file)).unwrap()).unwrap();
        conform::conforms(&message);
        assert_eq!(format!("{}.json", message["type"].as_str().unwrap()), file);
        kinds.insert(file);
    }
    let schemas = reference().join("../schemas/events");
    assert_eq!(kinds, files(&schemas, &schemas));
}

/// Each `<!-- example: <path> -->` line is followed by a code block that holds that file of
/// `docs/reference/examples/` exactly, and every example file is shown somewhere.
#[test]
fn the_reference_pages_show_every_example_file_as_it_is() {
    let mut shown = BTreeSet::new();
    for entry in fs::read_dir(reference()).unwrap() {
        let page = entry.unwrap().path();
        if page.extension().is_none_or(|extension| extension != "md") {
            continue;
        }
        let text = fs::read_to_string(&page).unwrap();
        let mut lines = text.lines();
        while let Some(line) = lines.next() {
            let Some(path) = line
                .strip_prefix("<!-- example: ")
                .and_then(|rest| rest.strip_suffix(" -->"))
            else {
                continue;
            };
            let fence = lines.next().unwrap_or_default();
            assert!(
                fence.starts_with("```"),
                "{}: no code block after {line}",
                page.display()
            );
            let block: Vec<&str> = lines.by_ref().take_while(|line| *line != "```").collect();
            let file = fs::read_to_string(examples().join(path))
                .unwrap_or_else(|_| panic!("{}: no example file {path}", page.display()));
            assert_eq!(
                block.join("\n") + "\n",
                file,
                "{}: the block for {path} differs from the file",
                page.display()
            );
            shown.insert(path.to_owned());
        }
    }
    let unshown: Vec<_> = files(&examples(), &examples())
        .difference(&shown)
        .cloned()
        .collect();
    assert!(unshown.is_empty(), "no reference page shows {unshown:?}");
}

/// The commands `priory` prints with no arguments, one per line, without the leading `usage:`
/// and any trailing note.
fn usage_lines() -> Vec<String> {
    let output = Command::new(env!("CARGO_BIN_EXE_priory")).output().unwrap();
    String::from_utf8(output.stderr)
        .unwrap()
        .lines()
        .map(|line| {
            let line = line.trim_start_matches("usage:").trim();
            line.split("    ").next().unwrap().to_string()
        })
        .collect()
}

/// Every `priory ...` code span of `text`.
fn commands(text: &str) -> Vec<&str> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| span.starts_with("priory "))
        .collect()
}

#[test]
fn the_cli_page_lists_every_usage_line_and_nothing_else() {
    let page = fs::read_to_string(reference().join("cli.md")).unwrap();
    let usage = usage_lines();
    let documented = commands(&page);
    let missing: Vec<_> = usage
        .iter()
        .filter(|line| !documented.contains(&line.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "cli.md misses these usage lines: {missing:#?}"
    );

    for command in documented {
        let verb = words(command).nth(1).unwrap();
        let lines: Vec<_> = usage
            .iter()
            .filter(|line| verbs(line).any(|word| word == verb))
            .collect();
        assert!(
            !lines.is_empty(),
            "cli.md names `{command}`, whose verb `priory` does not have"
        );
        for flag in words(command).filter(|word| word.starts_with("--")) {
            assert!(
                lines
                    .iter()
                    .any(|line| words(line).any(|word| word == flag)),
                "cli.md names `{command}`, whose verb {verb} takes no {flag}"
            );
        }
    }
}

/// The verbs a usage line is for: its first word, or each word of a leading `(a | b)` group.
fn verbs(line: &str) -> impl Iterator<Item = &str> {
    let rest = line.trim_start_matches("priory ");
    let verbs = match rest.strip_prefix('(') {
        Some(group) => group.split(')').next().unwrap(),
        None => rest.split(' ').next().unwrap(),
    };
    words(verbs)
}

/// The words of a command, without its brackets, bars and placeholders' punctuation.
fn words(command: &str) -> impl Iterator<Item = &str> {
    command
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|word| !word.is_empty())
}
