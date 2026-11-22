//! `priory answer` and `priory dismiss` name the revision they miss, instead of the whole usage.

use std::path::Path;
use std::process::Command;

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
        let output = Command::new(env!("CARGO_BIN_EXE_priory"))
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
