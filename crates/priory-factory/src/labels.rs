//! A judge scorer's labeled cases: a request, a result and a diff, each with the verdict a person
//! gave it. `priory score --calibrate` runs the judge on them, and `priory check` counts them.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// `scorers/<name>.eval.toml`: the cases a person labeled for a judge scorer.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "judge cases")]
pub struct Labels {
    /// The labeled cases: at least 10, of which at least 3 `pass` and 3 `fail`.
    pub cases: Vec<Labeled>,
}

/// One labeled case: what the judge reads, and the verdict a person gave it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Labeled {
    /// A name, unique in the file.
    pub name: String,
    /// What the task was asked.
    pub request: String,
    /// The run's result, with its `outcome`, `summary` and other fields.
    #[serde(default)]
    #[schemars(schema_with = "table_schema")]
    pub result: BTreeMap<String, toml::Value>,
    /// The change, as `git diff` prints it.
    #[serde(default)]
    pub diff: String,
    /// The right verdict.
    pub verdict: Label,
    /// Why the person gave that verdict, for a reader.
    pub why: Option<String>,
}

/// A person's verdict on a labeled case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Label {
    Pass,
    Fail,
}

impl Label {
    pub fn name(self) -> &'static str {
        match self {
            Label::Pass => "pass",
            Label::Fail => "fail",
        }
    }
}

fn table_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "object" })
}

/// The fewest labeled cases a judge needs, and the fewest of each verdict among them.
pub const CASES: usize = 10;
pub const EACH: usize = 3;

/// Reads the labeled cases at `path`, and refuses a file that breaks the rule [`CASES`] and
/// [`EACH`] set or names two cases alike.
pub fn load(path: &Path) -> Result<Labels, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let labels: Labels = toml::from_str(&text).map_err(|error| error.to_string())?;
    let mut seen = BTreeSet::new();
    if let Some(twice) = labels.cases.iter().find(|case| !seen.insert(&case.name)) {
        return Err(format!("two cases named {}", twice.name));
    }
    let count = |label| {
        labels
            .cases
            .iter()
            .filter(|case| case.verdict == label)
            .count()
    };
    let (pass, fail) = (count(Label::Pass), count(Label::Fail));
    if labels.cases.len() < CASES || pass < EACH || fail < EACH {
        return Err(format!(
            "a judge needs at least {CASES} labeled cases, {EACH} of them pass and {EACH} fail: this file has {} ({pass} pass, {fail} fail)",
            labels.cases.len()
        ));
    }
    Ok(labels)
}

/// The triage decision point's `*.eval.toml`: feedback items a person sorted.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "triage cases")]
pub struct TriageCases {
    /// The labeled cases: at least 10, of which at least 3 of each class.
    pub cases: Vec<TriageCase>,
}

/// One feedback item and the class a person gave it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TriageCase {
    /// A name, unique in the file.
    pub name: String,
    /// The event type, such as `github.pull_request_review_comment`; a pull request comment when
    /// unset.
    pub kind: Option<String>,
    /// What the person wrote.
    pub body: String,
    /// The right class.
    pub label: Class,
    /// Why the person gave that class, for a reader.
    pub why: Option<String>,
}

/// How an agent treats a feedback item, as the result tool's `feedback` field names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    Trivial,
    NothingToDo,
    PersonsCall,
}

impl Class {
    pub const ALL: [Class; 3] = [Class::Trivial, Class::NothingToDo, Class::PersonsCall];

    pub fn name(self) -> &'static str {
        match self {
            Class::Trivial => "trivial",
            Class::NothingToDo => "nothing-to-do",
            Class::PersonsCall => "persons-call",
        }
    }
}

/// Reads the triage cases at `path`, and refuses a file with fewer than [`CASES`] cases, fewer than
/// [`EACH`] of a class, or two cases alike.
pub fn load_triage(path: &Path) -> Result<TriageCases, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    let labels: TriageCases = toml::from_str(&text).map_err(|error| error.to_string())?;
    let mut seen = BTreeSet::new();
    if let Some(twice) = labels.cases.iter().find(|case| !seen.insert(&case.name)) {
        return Err(format!("two cases named {}", twice.name));
    }
    let counts: Vec<usize> = Class::ALL
        .iter()
        .map(|class| {
            labels
                .cases
                .iter()
                .filter(|case| case.label == *class)
                .count()
        })
        .collect();
    if labels.cases.len() < CASES || counts.iter().any(|count| *count < EACH) {
        return Err(format!(
            "triage needs at least {CASES} labeled cases, {EACH} of each class: this file has {} ({} trivial, {} nothing-to-do, {} persons-call)",
            labels.cases.len(),
            counts[0],
            counts[1],
            counts[2]
        ));
    }
    Ok(labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, text: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "priory-labels-{}-{name}.eval.toml",
            std::process::id()
        ));
        fs::write(&path, text).unwrap();
        path
    }

    fn cases(pass: usize, fail: usize) -> String {
        (0..pass + fail)
            .map(|n| {
                let verdict = if n < pass { "pass" } else { "fail" };
                format!(
                    "[[cases]]\nname = \"c{n}\"\nrequest = \"Add a flag.\"\ndiff = \"+flag\"\nverdict = \"{verdict}\"\n"
                )
            })
            .collect()
    }

    #[test]
    fn load_counts_the_cases_of_each_verdict() {
        let ok = file("ok", cases(5, 5).as_bytes());
        let few = file("few", cases(7, 2).as_bytes());

        assert_eq!(load(&ok).unwrap().cases.len(), 10);
        assert_eq!(
            load(&few).unwrap_err(),
            "a judge needs at least 10 labeled cases, 3 of them pass and 3 fail: this file has 9 (7 pass, 2 fail)"
        );
        fs::remove_file(ok).unwrap();
        fs::remove_file(few).unwrap();
    }

    #[test]
    fn load_refuses_two_cases_with_one_name() {
        let text = format!("{}{}", cases(5, 5), cases(1, 0));
        let path = file("twice", text.as_bytes());

        assert_eq!(load(&path).unwrap_err(), "two cases named c0");
        fs::remove_file(path).unwrap();
    }

    fn triage(nothing: usize) -> String {
        [
            ("nothing-to-do", nothing),
            ("trivial", 3),
            ("persons-call", 3),
        ]
        .iter()
        .flat_map(|(label, count)| {
            (0..*count).map(move |n| {
                format!("[[cases]]\nname = \"{label}-{n}\"\nbody = \"b\"\nlabel = \"{label}\"\n")
            })
        })
        .collect()
    }

    #[test]
    fn load_triage_counts_the_cases_of_each_class() {
        let ok = file("triage-ok", triage(4).as_bytes());
        let few = file("triage-few", triage(2).as_bytes());

        assert_eq!(load_triage(&ok).unwrap().cases.len(), 10);
        assert_eq!(
            load_triage(&few).unwrap_err(),
            "triage needs at least 10 labeled cases, 3 of each class: this file has 8 (3 trivial, 2 nothing-to-do, 3 persons-call)"
        );
        fs::remove_file(ok).unwrap();
        fs::remove_file(few).unwrap();
    }

    proptest::proptest! {
        #[test]
        fn load_triage_reads_any_file_without_panicking(
            text in "[ -~\n]{0,300}",
            cut in 0usize..400,
            flip in proptest::collection::vec(proptest::prelude::any::<(usize, u8)>(), 0..8),
        ) {
            let _ = load_triage(&file("triage-any", text.as_bytes()));
            let good = triage(4).into_bytes();
            let _ = load_triage(&file("triage-cut", &good[..cut.min(good.len())]));
            let mut mutated = good;
            for (at, byte) in flip {
                let len = mutated.len();
                mutated[at % len] = byte;
            }
            let _ = load_triage(&file("triage-flipped", &mutated));
        }
    }

    proptest::proptest! {
        #[test]
        fn load_reads_any_file_without_panicking(
            text in "[ -~\n]{0,300}",
            cut in 0usize..400,
            flip in proptest::collection::vec(proptest::prelude::any::<(usize, u8)>(), 0..8),
        ) {
            let _ = load(&file("any", text.as_bytes()));
            let good = cases(5, 5).into_bytes();
            let _ = load(&file("cut", &good[..cut.min(good.len())]));
            let mut mutated = good;
            for (at, byte) in flip {
                let len = mutated.len();
                mutated[at % len] = byte;
            }
            let path = file("mutated", &mutated);
            let _ = load(&path);
            fs::remove_file(path).unwrap();
        }
    }
}
