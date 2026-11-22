//! The `paths` globs of a skill or memory: which files of a task's repository bring it into a run.
//!
//! A glob is relative to the repository root and takes `*` and `?` within one part of a path,
//! `**` for any number of whole parts, and `{a,b}` for either text.

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `paths` as a frontmatter writes it: one glob, or a list of them.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Globs {
    One(String),
    Many(Vec<String>),
}

impl Globs {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            Globs::One(glob) => vec![glob],
            Globs::Many(globs) => globs,
        }
    }
}

/// A skill or memory with `paths`, as a run's `PRIORY_PATHS` file lists it for the hook.
#[derive(Serialize, Deserialize)]
pub struct Scoped {
    /// `skill` or `memory`.
    pub kind: String,
    pub name: String,
    /// The file in the run's sealed plugin folder or memory snapshot.
    pub file: String,
    pub globs: Vec<String>,
    /// The file's text, which the hook adds to the run's context.
    pub text: String,
}

/// Why `priory check` refuses `glob`, if it does.
pub fn problem(glob: &str) -> Option<String> {
    if glob.trim().is_empty() {
        return Some("a path glob is empty".into());
    }
    let why = |why: &str| Some(format!("the path glob `{glob}` {why}"));
    if glob.starts_with('/') {
        return why("starts with /: a glob is relative to the repository root");
    }
    if let Some(c) = glob.chars().find(|c| matches!(c, '[' | ']' | '\\')) {
        return why(&format!("holds `{c}`: a glob takes *, **, ? and {{a,b}}"));
    }
    let mut open = false;
    for c in glob.chars() {
        match (c, open) {
            ('{', true) | ('}', false) => return why("nests braces or leaves one open"),
            ('{', false) => open = true,
            ('}', true) => open = false,
            _ => {}
        }
    }
    if open {
        return why("nests braces or leaves one open");
    }
    for part in glob.split('/') {
        if part.is_empty() {
            return why("has an empty part, as in a//b or a trailing /");
        }
        if part == "." || part == ".." {
            return why("has a `.` or `..` part");
        }
        if part.contains("**") && part != "**" {
            return why("puts ** inside a part: ** stands alone between slashes");
        }
    }
    None
}

/// The anchored regular expression of a glob `problem` accepts.
fn regex(glob: &str) -> Option<Regex> {
    let mut pattern = String::from("^");
    let mut rest = glob;
    let mut braced = false;
    while let Some(c) = rest.chars().next() {
        if let Some(after) = rest.strip_prefix("**") {
            match after.strip_prefix('/') {
                Some(after) => {
                    pattern.push_str("(?:[^/]+/)*");
                    rest = after;
                }
                None => {
                    pattern.push_str(".*");
                    rest = after;
                }
            }
            continue;
        }
        match c {
            '*' => pattern.push_str("[^/]*"),
            '?' => pattern.push_str("[^/]"),
            '{' => {
                braced = true;
                pattern.push_str("(?:");
            }
            '}' => {
                braced = false;
                pattern.push(')');
            }
            ',' if braced => pattern.push('|'),
            c => pattern.push_str(&regex::escape(&c.to_string())),
        }
        rest = &rest[c.len_utf8()..];
    }
    pattern.push('$');
    Regex::new(&pattern).ok()
}

/// Whether `path`, relative to the repository root with `/` between its parts, matches `glob`.
/// A glob `problem` refuses matches nothing.
pub fn matches(glob: &str, path: &str) -> bool {
    problem(glob).is_none() && regex(glob).is_some_and(|regex| regex.is_match(path))
}

#[cfg(test)]
mod tests {
    use super::{matches, problem};
    use proptest::prelude::*;

    #[test]
    fn a_star_stays_in_one_part_and_a_double_star_crosses_parts() {
        assert!(matches("src/*.rs", "src/main.rs"));
        assert!(!matches("src/*.rs", "src/bin/main.rs"));
        assert!(matches("src/**/*.rs", "src/main.rs"));
        assert!(matches("src/**/*.rs", "src/bin/deep/main.rs"));
        assert!(matches("**/*.md", "README.md"));
        assert!(matches("**/*.md", "docs/a/b.md"));
        assert!(matches("docs/**", "docs/a/b.md"));
        assert!(!matches("docs/**", "doc/a.md"));
        assert!(matches("**", "any/path"));
        assert!(matches("crates/?/x", "crates/a/x"));
        assert!(!matches("crates/?/x", "crates/ab/x"));
    }

    #[test]
    fn braces_take_either_text_and_dots_are_literal() {
        assert!(matches("docs/*.{md,txt}", "docs/a.txt"));
        assert!(!matches("docs/*.{md,txt}", "docs/a.rs"));
        assert!(matches("{src,tests}/**/*.rs", "tests/x.rs"));
        assert!(!matches("a.md", "aXmd"));
        assert!(matches("a,b", "a,b"));
        assert!(matches("x+(y)", "x+(y)"));
    }

    #[test]
    fn a_glob_the_hook_cannot_match_is_refused() {
        for glob in [
            "", "/etc", "a/../b", "./a", "a//b", "a/", "x**", "[ab]", "{a", "a}",
        ] {
            assert!(problem(glob).is_some(), "{glob}");
            assert!(!matches(glob, glob), "{glob}");
        }
        assert_eq!(problem("src/**/*.{rs,toml}"), None);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        fn any_glob_and_path_return_without_panicking(glob in ".{0,40}", path in ".{0,40}") {
            let _ = problem(&glob);
            let _ = matches(&glob, &path);
        }

        #[test]
        fn a_glob_without_wildcards_matches_only_itself(
            path in "[a-z0-9._-]{1,8}(/[a-z0-9_-][a-z0-9._-]{0,7}){0,3}",
        ) {
            prop_assume!(problem(&path).is_none());
            prop_assert!(matches(&path, &path));
            let longer = format!("{path}x");
            prop_assert!(!matches(&path, &longer));
        }
    }
}
