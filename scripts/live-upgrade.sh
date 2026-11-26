#!/bin/sh
# Builds the last release and this branch with one more, additive, migration, then runs the
# live-run upgrade test between them: `scripts/live-upgrade.sh [TAG]`. CI runs it.
set -eu
root=$(git rev-parse --show-toplevel)
tag=${1:-$(git -C "$root" describe --tags --abbrev=0 --match 'v*' origin/main 2>/dev/null || git -C "$root" describe --tags --abbrev=0 --match 'v*')}
work=$(mktemp -d)
trap 'git -C "$root" worktree remove --force "$work/old" 2>/dev/null; rm -rf "$work"' EXIT

git -C "$root" worktree add --detach "$work/old" "$tag" >/dev/null
cargo build -q -p priory --manifest-path "$work/old/Cargo.toml" --target-dir "$root/target/live-upgrade/old"

# A copy of this branch whose state gains a table, as an additive release does.
mkdir "$work/new"
git -C "$root" ls-files -z | (cd "$root" && xargs -0 cp --parents -t "$work/new")
python3 - "$work/new/crates/priory-core/src/state.rs" <<'PY'
import re, sys
path = sys.argv[1]
text = open(path).read()
version = int(re.search(r"const VERSION: i64 = (\d+);", text).group(1))
new = version + 1
text = text.replace(f"const VERSION: i64 = {version};", f"const VERSION: i64 = {new};", 1)
text = text.replace("/// Each schema change in order", f'''const SCHEMA_{new}: &str = "
CREATE TABLE IF NOT EXISTS live_upgrade (id INTEGER PRIMARY KEY, note TEXT);
ALTER TABLE inbox ADD COLUMN live_upgrade TEXT DEFAULT 'x';
PRAGMA user_version = {new};
";

/// Each schema change in order''', 1)
text = re.sub(r"(const MIGRATIONS: \[&str; VERSION as usize\] = \[.*?)(\n\];)", lambda m: m.group(1) + f" SCHEMA_{new}," + m.group(2), text, count=1, flags=re.S)
text = re.sub(r"(const KINDS: \[Kind; VERSION as usize - UNDECLARED\] = \[)(.*?)(\];)", lambda m: m.group(1) + m.group(2) + ("" if m.group(2).strip() == "" or m.group(2).rstrip().endswith(",") else ", ") + "Kind::Additive" + m.group(3), text, count=1, flags=re.S)
open(path, "w").write(text)
PY
cargo build -q -p priory --manifest-path "$work/new/Cargo.toml" --target-dir "$root/target/live-upgrade/new"

PRIORY_TEST_OLD="$root/target/live-upgrade/old/debug/priory" \
PRIORY_TEST_NEW="$root/target/live-upgrade/new/debug/priory" \
  cargo test -p priory --manifest-path "$root/Cargo.toml" --test runtime -- --ignored --exact \
  live_upgrade::processes_that_outlive_an_upgrade_and_its_rollback_keep_working
