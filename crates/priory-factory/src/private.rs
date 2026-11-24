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
    /// Literal strings, matched in any case and as whole words.
    #[serde(default)]
    pub text: Vec<String>,
    /// Regexes, in the regex crate's syntax.
    #[serde(default)]
    pub regex: Vec<String>,
}

impl Term {
    fn compile(&self) -> Result<Vec<Regex>, regex::Error> {
        let literals = self.text.iter().map(|text| {
            let word = |c: char| c.is_alphanumeric() || c == '_';
            let edge = |at_word: bool| if at_word { r"\b" } else { "" };
            let start = edge(text.starts_with(word));
            let end = edge(text.ends_with(word));
            RegexBuilder::new(&format!("{start}{}{end}", regex::escape(text)))
                .case_insensitive(true)
                .build()
        });
        let patterns = self.regex.iter().map(|pattern| Regex::new(pattern));
        literals.chain(patterns).collect()
    }
}

/// Reads `[private]` with an error that repeats nothing of it, since it may hold a term.
pub(crate) fn private<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Private, D::Error> {
    serde_json::Value::deserialize(input)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| {
            serde::de::Error::custom(
                "`[private]` takes `repos`, an array of strings, and `terms`, an array of tables of a `label` string and `text` and `regex` arrays of strings",
            )
        })
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

    /// What `priory check` warns about, with no term in it.
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        for (label, regex) in self.compile().unwrap_or_default() {
            for (at, repo) in self.repos.iter().enumerate() {
                if regex.is_match(repo) {
                    warnings.push(format!(
                        "`[private]` term `{label}` matches entry {} of `repos`: a text that names that repository is refused",
                        at + 1
                    ));
                }
            }
        }
        warnings
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
