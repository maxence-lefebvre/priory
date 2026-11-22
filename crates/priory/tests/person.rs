//! The verbs only a person runs refuse inside a run, whose pane sets `PRIORY_SESSION`.

use std::path::Path;
use std::process::Command;

#[test]
fn a_person_verb_refuses_inside_a_run() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("person");
    priory_runtime::runtime::remove_sealed(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for args in [
        &["answer", "--revision", "1", "12", "approve"][..],
        &["dismiss", "12"],
        &["release", "7"],
        &["give", "github:owner/repo#1", "b"],
        &["eval", "prompts/review.md"],
        &["bench", "replay", "cases/eng-412", "--budget", "5"],
        &["report", "pick", "3", "skip", "--expected", "2"],
        &["label", "7", "wrong", "--clear"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_priory"))
            .args(args)
            .env("PRIORY_SESSION", "6f1c0b9e-0d2a-4f43-9a51-3c2e8e1b7d10")
            .env("PRIORY_DB", dir.join("priory.db"))
            .env("PRIORY_HOME", dir.join("home"))
            .env("XDG_RUNTIME_DIR", &dir)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!(
                "priory {}: only a person runs it, and this shell is a run's (PRIORY_SESSION is set)\n",
                args[0]
            ),
            "{args:?}"
        );
    }
}
