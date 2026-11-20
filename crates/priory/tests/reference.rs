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

#[test]
fn the_example_factory_passes_check() {
    let output = Command::new(env!("CARGO_BIN_EXE_priory"))
        .arg("check")
        .arg(examples().join("factory"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
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
        let _ = fs::remove_dir_all(&dir);
        copy(&examples().join("factory"), &dir);
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
