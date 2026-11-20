use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn conforms(message: &Value) {
    let kind = message["type"].as_str().expect("a message type");
    matches(&format!("events/{kind}.json"), message);
}

pub fn matches(schema: &str, value: &Value) {
    let errors = errors(schema, value);
    assert!(
        errors.is_empty(),
        "breaks docs/schemas/{schema}: {errors:#?}\n{value:#}"
    );
}

pub fn errors(schema: &str, value: &Value) -> Vec<String> {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/schemas")
        .join(schema);
    let text = fs::read_to_string(&file).unwrap_or_else(|_| panic!("no schema {schema}"));
    let validator = jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap();
    validator
        .iter_errors(value)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect()
}
