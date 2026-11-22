//! `priory answer` and `priory dismiss` name the revision they miss, instead of the whole usage.

#[allow(dead_code)]
mod harness;

use std::path::Path;

#[test]
fn an_answer_or_remote_dismiss_without_a_revision_names_the_flag() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("answer");
    priory_runtime::runtime::remove_sealed(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (args, ask) in [
        (&["answer", "12", "approve"][..], "12"),
        (&["answer", "b:12", "approve"], "b:12"),
        (&["dismiss", "b:12"], "b:12"),
    ] {
        let output = harness::priory()
            .args(args)
            .env("PRIORY_DB", dir.join("priory.db"))
            .env("PRIORY_HOME", dir.join("home"))
            .env("XDG_RUNTIME_DIR", &dir)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!(
                "priory {}: ask {ask} needs --revision N, the revision `priory asks` prints for it\n",
                args[0]
            ),
            "{args:?}"
        );
    }
}

#[test]
fn an_answer_without_a_revision_prints_the_command_with_the_current_revision() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("answer-command");
    priory_runtime::runtime::remove_sealed(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let priory = |args: &[&str]| {
        harness::priory()
            .args(args)
            .env_remove("PRIORY_FACTORY")
            .env("PRIORY_DB", dir.join("priory.db"))
            .env("PRIORY_HOME", dir.join("home"))
            .env("HERDR_BUS_DB", dir.join("bus.db"))
            .env("XDG_RUNTIME_DIR", &dir)
            .output()
            .unwrap()
    };
    for question in ["Ship it?", "Ship it now?"] {
        let fields = format!(r#"{{"question": "{question}", "options": ["ok", "not yet"]}}"#);
        let asked = priory(&["ask", "smoke.1", &fields]);
        assert!(asked.status.success(), "{asked:?}");
    }

    for (args, run) in [
        (
            &["answer", "smoke.1", "ok"][..],
            "priory answer --revision 2 smoke.1 ok",
        ),
        (
            &["answer", "smoke.1", "not yet"],
            "priory answer --revision 2 smoke.1 'not yet'",
        ),
        (
            &["answer", "--run", "smoke.1", "it's fine"],
            r"priory answer --revision 2 --run smoke.1 'it'\''s fine'",
        ),
    ] {
        let output = priory(args);

        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!(
                "priory answer: ask smoke.1 needs --revision N, the revision `priory asks` prints for it; run: {run}\n"
            ),
            "{args:?}"
        );
    }
}
