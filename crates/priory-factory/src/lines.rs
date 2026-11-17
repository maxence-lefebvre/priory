use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;

/// Where each top-level field and each step of a trigger or workflow file starts.
#[derive(Debug, Default)]
pub struct Lines {
    fields: BTreeMap<String, usize>,
    steps: Vec<usize>,
}

impl Lines {
    pub fn field(&self, name: &str) -> Option<usize> {
        self.fields.get(name).copied()
    }

    pub fn step(&self, index: usize) -> Option<usize> {
        self.steps.get(index).copied()
    }
}

#[derive(Deserialize)]
struct Steps<S> {
    #[serde(default = "Vec::new")]
    steps: Vec<S>,
}

pub fn toml(text: &str, first_line: usize) -> Option<Lines> {
    type Value = toml::Spanned<IgnoredAny>;
    let line =
        |value: &Value| crate::factory::line_column(text, value.span().start).0 + first_line - 1;
    let fields: BTreeMap<String, Value> = toml::from_str(text).ok()?;
    let steps: Steps<Value> = toml::from_str(text).ok()?;
    Some(Lines {
        fields: fields
            .iter()
            .map(|(name, at)| (name.clone(), line(at)))
            .collect(),
        steps: steps.steps.iter().map(line).collect(),
    })
}

pub fn yaml(text: &str, first_line: usize) -> Option<Lines> {
    type Value = serde_saphyr::Spanned<IgnoredAny>;
    let line = |value: &Value| value.referenced.line() as usize + first_line - 1;
    let fields: BTreeMap<String, Value> = serde_saphyr::from_str(text).ok()?;
    let steps: Steps<Value> = serde_saphyr::from_str(text).ok()?;
    Some(Lines {
        fields: fields
            .iter()
            .map(|(name, at)| (name.clone(), line(at)))
            .collect(),
        steps: steps.steps.iter().map(line).collect(),
    })
}
