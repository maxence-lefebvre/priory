//! `priory ask NAME JSON` takes text from outside sessions: it stores the ask or refuses it.

use proptest::prelude::*;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn priory(dir: &Path, name: &str, fields: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_priory"))
        .args(["ask", name, fields])
        .env("PRIORY_DB", dir.join("priory.db"))
        .env("PRIORY_HOME", dir.join("home"))
        .env("HERDR_BUS_DB", dir.join("bus.db"))
        .output()
        .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn any_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        "\\PC{0,60}".prop_map(Value::from),
        proptest::collection::vec(any::<char>(), 0..3000)
            .prop_map(|chars| Value::from(chars.into_iter().collect::<String>())),
    ];
    leaf.prop_recursive(4, 32, 5, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            proptest::collection::vec(("[a-z_]{1,12}", inner), 0..4)
                .prop_map(|pairs| Value::Object(pairs.into_iter().collect())),
        ]
    })
}

fn fields() -> impl Strategy<Value = String> {
    let names = prop_oneof![
        Just("question".to_owned()),
        Just("title".to_owned()),
        Just("options".to_owned()),
        Just("text".to_owned()),
        Just("command".to_owned()),
        "[a-z_]{1,12}",
    ];
    prop_oneof![
        "\\PC{0,200}",
        any_value().prop_map(|value| value.to_string()),
        proptest::collection::vec((names, any_value()), 0..8)
            .prop_map(|pairs| Value::Object(pairs.into_iter().collect()).to_string()),
        (1..5000usize).prop_map(|depth| format!("{}0{}", "[".repeat(depth), "]".repeat(depth))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]

    #[test]
    fn an_ask_of_any_name_and_shape_is_stored_or_refused_without_panicking(
        name in prop_oneof!["[A-Za-z0-9._-]{1,120}", "\\PC{0,130}"],
        fields in fields(),
    ) {
        let dir = scratch("ask-fuzz");
        let output = priory(&dir, &name, &fields);
        let stderr = String::from_utf8_lossy(&output.stderr);
        prop_assert!(!stderr.contains("panicked"), "{stderr}");
        prop_assert!(output.status.code().is_some_and(|code| code != 101), "{output:?}");
    }
}

#[test]
fn a_question_stores_the_ask() {
    let dir = scratch("ask-ok");
    let output = priory(
        &dir,
        "cache.ship",
        &json!({ "question": "Ship?" }).to_string(),
    );
    assert!(output.status.success(), "{output:?}");
}
