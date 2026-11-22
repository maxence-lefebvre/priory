use std::fs;
use std::path::Path;

/// The values of every `KEY=` line in a unit file.
fn values<'a>(unit: &'a str, key: &str) -> Vec<&'a str> {
    let prefix = format!("{key}=");
    unit.lines()
        .filter_map(|line| line.trim().strip_prefix(prefix.as_str()))
        .collect()
}

/// Each unit in `contrib/systemd/` starts only once its environment file exists, so a package or
/// home-manager switch that links the units does not start them before a person writes the file.
#[test]
fn each_contrib_unit_waits_for_its_environment_file() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contrib/systemd");
    for name in ["priory-daemon.service", "priory-intake.service"] {
        let unit = fs::read_to_string(dir.join(name)).unwrap();
        let files = values(&unit, "EnvironmentFile");
        assert_eq!(files, ["%h/.config/priory/env"], "{name}");
        assert_eq!(values(&unit, "ConditionPathExists"), files, "{name}");
    }
}
