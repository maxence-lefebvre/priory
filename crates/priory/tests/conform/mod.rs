use serde_json::Value;
use std::fs;
use std::path::Path;

/// Panics unless `message` validates against the committed schema of its type in
/// `docs/schemas/events/`.
pub fn conforms(message: &Value) {
    let kind = message["type"].as_str().expect("a message type");
    matches(&format!("events/{kind}.json"), message);
}

/// Panics unless `value` validates against `docs/schemas/<schema>`.
pub fn matches(schema: &str, value: &Value) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/schemas")
        .join(schema);
    let text = fs::read_to_string(&file).unwrap_or_else(|_| panic!("no schema {schema}"));
    let validator = jsonschema::validator_for(&serde_json::from_str(&text).unwrap()).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|error| format!("{}: {error}", error.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "breaks docs/schemas/{schema}: {errors:#?}\n{value:#}"
    );
}
