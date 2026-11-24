//! `[private]` in `factory.toml`: the terms no run may publish to the repositories it lists.

use regex::{Regex, RegexBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The factory's private terms, and the repositories they must stay out of.
#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Private {
    /// The GitHub repositories, each `owner/name`, that must never hold a term.
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub terms: Vec<Term>,
}

/// Private terms under one label, which a refusal names instead of the term.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Term {
    pub label: String,
    /// Literal strings, matched in any case.
    #[serde(default)]
    pub text: Vec<String>,
    /// Regexes, in the regex crate's syntax.
    #[serde(default)]
    pub regex: Vec<String>,
}

impl Term {
    fn compile(&self) -> Result<Vec<Regex>, regex::Error> {
        let literals = self.text.iter().map(|text| {
            RegexBuilder::new(&regex::escape(text))
                .case_insensitive(true)
                .build()
        });
        let patterns = self.regex.iter().map(|pattern| Regex::new(pattern));
        literals.chain(patterns).collect()
    }
}

impl Private {
    /// Each term's label and what it matches. An error names the label alone.
    pub fn compile(&self) -> Result<Vec<(String, Regex)>, String> {
        let mut all = Vec::new();
        for term in &self.terms {
            let compiled = term.compile().map_err(|error| {
                let error = error.to_string();
                let reason = error.lines().last().unwrap_or_default();
                format!(
                    "`[private]` term `{}` has a regex that does not compile: {reason}",
                    term.label
                )
            })?;
            all.extend(
                compiled
                    .into_iter()
                    .map(|regex| (term.label.clone(), regex)),
            );
        }
        Ok(all)
    }

    /// Whether the GitHub `owner/name` is listed, in any case.
    pub fn lists(&self, repo: &str) -> bool {
        self.repos
            .iter()
            .any(|listed| listed.eq_ignore_ascii_case(repo))
    }

    /// What `priory check` reports, with no term in it.
    pub fn problems(&self) -> Vec<String> {
        let mut problems: Vec<String> = self
            .repos
            .iter()
            .filter(|repo| priory_runtime::runtime::check_repo(repo).is_err())
            .map(|repo| format!("`[private] repos` `{repo}` needs to be `owner/name`"))
            .collect();
        for (at, term) in self.terms.iter().enumerate() {
            let label = &term.label;
            if label.trim().is_empty() {
                problems.push(format!("`[private]` term {} needs a label", at + 1));
                continue;
            }
            if self.terms[..at].iter().any(|other| other.label == *label) {
                problems.push(format!("`[private]` label `{label}` is used twice"));
            }
            if term.text.is_empty() && term.regex.is_empty() {
                problems.push(format!(
                    "`[private]` term `{label}` needs `text` or `regex`"
                ));
            }
        }
        match self.compile() {
            Err(problem) => problems.push(problem),
            Ok(terms) => {
                for (label, regex) in &terms {
                    if regex.is_match("") {
                        problems.push(format!("`[private]` term `{label}` matches empty text"));
                    } else if let Some(at) = self
                        .terms
                        .iter()
                        .position(|term| regex.is_match(&term.label))
                    {
                        problems.push(format!(
                            "`[private]` label {} holds a private term, and refusals print labels",
                            at + 1
                        ));
                    }
                }
            }
        }
        problems
    }
}
