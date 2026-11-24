#!/bin/sh
# Seeds a scratch state with synthetic data and serves it on 127.0.0.1:7391 for 15 minutes.
# Usage: scripts/demo/serve.sh DIR [PRIORY_BIN]
# DIR is a new or scratch folder. PRIORY_BIN defaults to target/debug/priory.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
dir=${1:?usage: serve.sh DIR [PRIORY_BIN]}
bin=${2:-$root/target/debug/priory}
mkdir -p "$dir/home" "$dir/factory"
cp -R "$root/examples/batches/." "$dir/factory/"
cp "$here/prices.toml" "$dir/factory/prices.toml"
# seed.sql's runners: this machine and the two whose tasks and asks it mirrors
runners="$dir/factory/runners"
mv "$runners/my-machine.toml" "$runners/workstation.toml"
cp "$runners/workstation.toml" "$runners/laptop.toml"
cp "$runners/workstation.toml" "$runners/desktop.toml"
sed -i 's/^name = .*/name = "shop"/' "$dir/factory/factory.toml"
# the memory a seeded candidate replaces
mkdir -p "$dir/factory/memories"
printf -- '---\nname: release-day\ndescription: When releases go out. Read it before you plan or merge a change near a release.\n---\nReleases go out on Tuesdays. Merge nothing risky on a Monday afternoon.\n' > "$dir/factory/memories/release-day.md"
# the plan document of task 14's worktree, which its approve-plan ask shows
mkdir -p "$dir/home/worktrees/acme/shop/eng-270/.priory"
cp "$here/brief.md" "$dir/home/worktrees/acme/shop/eng-270/.priory/brief.md"
printf '\n[serve]\nlisten = "127.0.0.1:7391"\ntoken_env = "PRIORY_TOKEN"\n' >> "$dir/factory/factory.toml"
export PRIORY_DB="$dir/state.db" PRIORY_HOME="$dir/home" PRIORY_TOKEN=demo-token
[ ! -e "$PRIORY_DB" ] || { echo "serve.sh: $PRIORY_DB exists; give a new folder" >&2; exit 1; }
"$bin" tasks --json > /dev/null # any verb creates and migrates the state
sqlite3 "$PRIORY_DB" < "$here/seed.sql"
exec timeout 900 "$bin" serve "$dir/factory"
