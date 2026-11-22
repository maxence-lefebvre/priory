//! The checks a repository's default branch requires before a merge, read with `gh`.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

use crate::runtime::{check_repo, run};

/// The default branch of a repository and the checks a merge into it needs, empty when none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Required {
    pub branch: String,
    pub checks: Vec<String>,
}

/// Reads the required checks of the default branch of the GitHub repository whose checkout is
/// `dir`: those of its branch rulesets, then those of its classic branch protection. Every call is
/// a read.
pub fn required_checks(dir: &Path) -> Result<Required, String> {
    // gh fills `{owner}` and `{repo}` from the checkout's git remote.
    let view = gh(dir, "repos/{owner}/{repo}")?;
    let (repo, branch) = repo_and_branch(&view)?;
    let path = branch
        .replace('%', "%25")
        .replace('#', "%23")
        .replace('?', "%3F");
    let rules = gh(dir, &format!("repos/{repo}/rules/branches/{path}"))?;
    let mut checks = ruleset_checks(&rules)?;
    if checks.is_empty() {
        let protection = gh(dir, &format!("repos/{repo}/branches/{path}"))?;
        checks = protection_checks(&protection)?;
    }
    Ok(Required { branch, checks })
}

/// A GET of `path` on GitHub's REST API, run in `dir`, with an error cut to its first line.
fn gh(dir: &Path, path: &str) -> Result<String, String> {
    run(Command::new("gh").current_dir(dir).args(["api", path])).map_err(|error| {
        let line = error.lines().next().unwrap_or_default();
        match line.char_indices().nth(200) {
            Some((end, _)) => format!("{}...", &line[..end]),
            None => line.to_owned(),
        }
    })
}

/// The `owner/name` and default branch in what `gh api repos/{owner}/{repo}` prints.
fn repo_and_branch(view: &str) -> Result<(String, String), String> {
    let value: Value =
        serde_json::from_str(view).map_err(|error| format!("gh api printed {view:?}: {error}"))?;
    let repo = value["full_name"]
        .as_str()
        .ok_or_else(|| format!("gh api printed no full_name: {view:?}"))?;
    check_repo(repo)?;
    let branch = value["default_branch"]
        .as_str()
        .filter(|branch| !branch.is_empty())
        .ok_or_else(|| format!("gh api printed no default_branch: {view:?}"))?;
    Ok((repo.to_owned(), branch.to_owned()))
}

/// The contexts of every `required_status_checks` rule in a branch's rules.
fn ruleset_checks(rules: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(rules)
        .map_err(|error| format!("gh api printed {rules:?}: {error}"))?;
    let rules = value
        .as_array()
        .ok_or_else(|| format!("gh api printed {rules:?}, not a list of rules"))?;
    let mut checks = Vec::new();
    for rule in rules
        .iter()
        .filter(|rule| rule["type"] == "required_status_checks")
    {
        let listed = rule["parameters"]["required_status_checks"].as_array();
        for check in listed.into_iter().flatten() {
            if let Some(context) = check["context"].as_str() {
                checks.push(context.to_owned());
            }
        }
    }
    checks.sort();
    checks.dedup();
    Ok(checks)
}

/// The contexts a branch's classic protection requires, in what `gh api repos/R/branches/B` prints.
fn protection_checks(branch: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(branch)
        .map_err(|error| format!("gh api printed {branch:?}: {error}"))?;
    if !value.is_object() {
        return Err(format!("gh api printed {branch:?}, not a branch"));
    }
    let required = &value["protection"]["required_status_checks"];
    let contexts = required["contexts"].as_array().into_iter().flatten();
    let checks = required["checks"].as_array().into_iter().flatten();
    let mut names: Vec<String> = contexts
        .filter_map(Value::as_str)
        .chain(checks.filter_map(|check| check["context"].as_str()))
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn the_view_names_the_repository_and_its_default_branch() {
        assert_eq!(
            repo_and_branch(r#"{"default_branch":"trunk","full_name":"a/b"}"#),
            Ok(("a".to_owned() + "/b", "trunk".to_owned()))
        );
        assert!(repo_and_branch(r#"{"default_branch":"","full_name":"a/b"}"#).is_err());
        assert!(repo_and_branch(r#"{"default_branch":"m","full_name":"../x"}"#).is_err());
    }

    #[test]
    fn only_required_status_check_rules_count() {
        let rules = r#"[{"type":"pull_request"},{"type":"required_status_checks","parameters":{"required_status_checks":[{"context":"test"},{"context":"lint"}]}}]"#;
        assert_eq!(
            ruleset_checks(rules),
            Ok(vec!["lint".to_owned(), "test".to_owned()])
        );
        assert_eq!(ruleset_checks("[]"), Ok(vec![]));
        assert!(ruleset_checks("{}").is_err());
    }

    #[test]
    fn an_unprotected_branch_requires_nothing() {
        assert_eq!(
            protection_checks(r#"{"protection":{"enabled":false}}"#),
            Ok(vec![])
        );
        assert_eq!(
            protection_checks(
                r#"{"protection":{"required_status_checks":{"contexts":["ci"],"checks":[{"context":"ci"}]}}}"#
            ),
            Ok(vec!["ci".to_owned()])
        );
    }

    proptest! {
        #[test]
        fn gh_output_parses_or_is_refused_without_panicking(
            text in r#"(\[|\{|"type"|"required_status_checks"|"parameters"|"context"|"protection"|"contexts"|"checks"|"full_name"|"default_branch"|:|,|\]|\}|"a/b"|"x"|[0-9]|null|\s){0,40}|.{0,80}"#
        ) {
            let _ = repo_and_branch(&text);
            let _ = ruleset_checks(&text);
            let _ = protection_checks(&text);
        }
    }
}
