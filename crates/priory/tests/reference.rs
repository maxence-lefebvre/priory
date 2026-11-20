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
