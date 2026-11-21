#!/usr/bin/env bash
# Runs a scratch two-runner factory across this machine and a second one reached over ssh, with one
# Postgres log on this machine that the second reaches through a reverse ssh tunnel. Both daemons
# use the fake `claude` and `gh` of crates/priory/tests/fake. docs/adding-a-second-machine.md
# explains the setup.
set -euo pipefail

usage() {
  cat <<'USAGE'
usage: scripts/second-machine.sh [--run] <command> <ssh-host> [args...]

  up                  build both priory binaries, start Postgres, the tunnel and both daemons
  down                stop both daemons, the tunnel and Postgres, and remove the scratch folders
  tunnel-up           start the reverse tunnel
  tunnel-down         stop the reverse tunnel
  start local|remote  start that machine's daemon
  stop local|remote   stop that machine's daemon
  priory local|remote ARGS...
                      run that machine's priory on the scratch factory, such as `tasks`

Without --run, it prints what it would do and touches nothing. Environment:
  SECOND_MACHINE_NAME   the scratch folder under the home on both machines, priory-second-machine
  SECOND_MACHINE_CLONE  the second machine's clone of this repository under its home, for this
                        script alone: `up` checks out SECOND_MACHINE_REF there and builds it.
                        priory-roger/priory by default
  SECOND_MACHINE_REF    what that clone builds, origin/main
USAGE
}

RUN=false
if [ "${1:-}" = --run ]; then
  RUN=true
  shift
fi
[ $# -ge 2 ] || { usage >&2; exit 2; }
command=$1 host=$2
shift 2

repo=$(cd "$(dirname "$0")/.." && pwd)
name=${SECOND_MACHINE_NAME:-priory-second-machine}
clone=${SECOND_MACHINE_CLONE:-priory-roger/priory}
ref=${SECOND_MACHINE_REF:-origin/main}
dir=$HOME/$name
container=priory-second-machine
origin=demo/two-machines
# Marks a folder this script made, so `down` removes nothing else.
marker=.second-machine

fail() {
  echo "second-machine: $*" >&2
  exit 1
}

# These values reach a remote shell and rm -rf, so they hold plain path characters only.
[[ $host =~ ^[A-Za-z0-9_.@-]+$ && $host != -* ]] || fail "ssh host \`$host\` is not user@host"
[[ $name =~ ^[A-Za-z0-9_-][A-Za-z0-9_.-]*$ ]] || fail "SECOND_MACHINE_NAME \`$name\` is not one folder name"
[[ $clone =~ ^[A-Za-z0-9_./-]+$ && $clone != *..* && $clone != /* ]] ||
  fail "SECOND_MACHINE_CLONE \`$clone\` is not a path under the home"
[[ $ref =~ ^[A-Za-z0-9_./-]+$ && $ref != -* ]] || fail "SECOND_MACHINE_REF \`$ref\` is not a git ref"

say() {
  printf '+ %s\n' "$*"
}

remote() {
  ssh -o BatchMode=yes "$host" "$1"
}

# Runs `script` with bash on one machine, in a cleared environment where S is the scratch folder
# and $S/env is sourced when it exists, with the remaining arguments as its own.
on() {
  local where=$1 script=$2
  shift 2
  local prelude='[ -e "$S/env" ] && . "$S/env"; '
  if [ "$where" = local ]; then
    env -i S="$dir" bash -c "$prelude$script" bash "$@"
  else
    remote "env -i S=\"\$HOME/$name\" bash -c $(printf %q "$prelude$script") bash${*:+$(printf ' %q' "$@")}"
  fi
}

free_gb() {
  df -Pk / | awk 'NR == 2 { print int($4 / 1048576) }'
}

preflight() {
  local here there
  here=$(free_gb)
  there=$(remote "df -Pk / | awk 'NR == 2 { print int(\$4 / 1048576) }'")
  echo "free disk: $here GB here, $there GB on $host"
  if [ "$here" -lt 20 ] || [ "$there" -lt 20 ]; then
    fail "under 20 GB free; stopping"
  fi
  if [ -e "$dir" ] || remote "test -e ~/$name"; then
    fail "~/$name exists on one of the machines; run down first"
  fi
}

# The factory both runners load: one runner file per machine, one agent and one workflow whose
# model step is followed by a person's approval.
write_factory() {
  local factory=$1 local_runner=$2 remote_runner=$3
  mkdir -p "$factory/runners" "$factory/agents/worker" "$factory/workflows"
  cat > "$factory/factory.toml" <<'EOF'
name = "two-machines"

[log]
backend = "postgres"
EOF
  # This runner's agents may take 1% of its memory, so a task it owns stays never-started and can move.
  cat > "$factory/runners/$local_runner.toml" <<'EOF'
platform = { os = "linux" }
agents = { memory = "1%" }
EOF
  cat > "$factory/runners/$remote_runner.toml" <<'EOF'
platform = { os = "linux" }
EOF
  cat > "$factory/agents/worker/agent.md" <<'EOF'
+++
description = "Does one small piece of work."
harness = "claude-code"
model = "haiku"
memory = "1GB"
+++
# Worker

Do the task, then call `result` with the outcome `worked`.
EOF
  cat > "$factory/workflows/work.toml" <<'EOF'
start = "work"

[[steps]]
id = "work"
decides = "model"
agent = "worker"
prompt = "Do the task."
effect = "none"
next = { worked = "approve" }

[[steps]]
id = "approve"
decides = "human"
effect = "none"
options = ["approved", "rejected"]
next = { approved = "done", rejected = "stop" }
EOF
}

# The scratch folder on one machine: the factory, the fakes first on PATH, a bare origin that git
# serves for github.com/demo/two-machines, and the fake agent's turns.
write_scratch() {
  local scratch=$1 priory=$2 local_runner=$3 remote_runner=$4
  mkdir -p "$scratch"/{bin,fake,home,priory,tmux,origin/demo}
  write_factory "$scratch/factory" "$local_runner" "$remote_runner"
  for fake in "$repo"/crates/priory/tests/fake/*; do
    cp "$fake" "$scratch/bin/"
  done
  ln -s "$priory" "$scratch/bin/priory"
  local work="$scratch/work"
  git init -q -b main "$work"
  echo two-machines > "$work/README.md"
  git -C "$work" add README.md
  git -C "$work" -c user.name=priory -c user.email=priory@localhost commit -qm init
  git clone -q --bare "$work" "$scratch/origin/$origin.git"
  rm -rf "$work"
  for _ in $(seq 20); do
    echo '{"name":"result","arguments":{"outcome":"worked","summary":"Worked"}}'
  done > "$scratch/fake/calls"
}

# The environment of each priory command, relative to the scratch folder `$S`.
env_file() {
  cat <<EOF
export PATH="\$S/bin:/usr/bin:/bin"
export HOME="\$S/home"
export PRIORY_HOME="\$S/priory"
export PRIORY_DB="\$S/priory.db"
export PRIORY_FACTORY="\$S/factory"
export FAKE_DIR="\$S/fake"
export TMUX_TMPDIR="\$S/tmux"
export XDG_RUNTIME_DIR="\$S/tmux"
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0="url.file://\$S/origin/.insteadOf"
export GIT_CONFIG_VALUE_0="https://github.com/"
export PRIORY_LOG_URL="postgresql://priory:$1@127.0.0.1:$2/priory?sslmode=disable"
EOF
}


plan() {
  say "check both machines have 20 GB free and no ~/$name"
  say "cargo build -p priory --features postgres, here and in ~/$clone on $host at $ref"
  say "docker run -d --name $container --env-file ~/$name/postgres.env -p 127.0.0.1:<random port>:5432 postgres:17-alpine"
  say "write ~/$name here and on $host: a factory with one runner file per hostname, the fakes, a bare origin, env"
  say "ssh -N -o ExitOnForwardFailure=yes -R 127.0.0.1:<port>:127.0.0.1:<port> $host, then check it listens on 127.0.0.1 only"
  say "priory daemon ~/$name/factory here, then on $host"
}

up() {
  $RUN || { plan; return; }
  preflight
  local local_runner remote_runner port password
  local_runner=$(hostname)
  remote_runner=$(remote hostname)
  [ "$local_runner" != "$remote_runner" ] || fail "both machines are named $local_runner; a runner file names one"
  port=$(shuf -i 20000-60000 -n 1)
  password=$(openssl rand -hex 24)
  trap 'echo "second-machine: up failed; run down to clean up" >&2' ERR
  echo "runners: $local_runner here, $remote_runner on $host; log port $port"

  cargo build --manifest-path "$repo/Cargo.toml" -p priory --features postgres
  remote "cd ~/$clone && git fetch -q origin && git checkout -q --detach $ref && PATH=\$HOME/.nix-profile/bin:\$HOME/.cargo/bin:\$PATH cargo build -p priory --features postgres"

  (umask 077 && mkdir "$dir")
  touch "$dir/$marker"
  (umask 077 && printf 'POSTGRES_USER=priory\nPOSTGRES_PASSWORD=%s\nPOSTGRES_DB=priory\n' "$password" > "$dir/postgres.env")
  docker run -d --name "$container" --env-file "$dir/postgres.env" -p "127.0.0.1:$port:5432" postgres:17-alpine > "$dir/container"
  echo "$port" > "$dir/port"
  # The image's first-init server listens on its socket only, so TCP answers once the real one runs.
  local tries=0
  until docker exec "$container" pg_isready -q -h 127.0.0.1 -U priory -d priory; do
    tries=$((tries + 1))
    [ "$tries" -lt 60 ] || fail "Postgres not ready after 60 s; run down to clean up"
    sleep 1
  done

  write_scratch "$dir" "$repo/target/debug/priory" "$local_runner" "$remote_runner"
  env_file "$password" "$port" | (umask 077 && cat > "$dir/env")
  local stage
  stage=$(mktemp -d)
  write_scratch "$stage/s" /nonexistent "$local_runner" "$remote_runner"
  rm "$stage/s/bin/priory"
  remote "umask 077 && mkdir ~/$name && touch ~/$name/$marker"
  tar -C "$stage/s" -cf - . | remote "tar -C ~/$name -xf - && ln -s ~/$clone/target/debug/priory ~/$name/bin/priory"
  rm -rf "$stage"
  env_file "$password" "$port" | remote "umask 077 && cat > ~/$name/env"

  tunnel_up
  start_daemon local
  start_daemon remote
  trap - ERR
}

tunnel_up() {
  $RUN || { say "ssh -N -o ExitOnForwardFailure=yes -R 127.0.0.1:<port>:127.0.0.1:<port> $host"; return; }
  local port bound
  port=$(cat "$dir/port")
  if [ -e "$dir/tunnel.pid" ] && kill -0 "$(cat "$dir/tunnel.pid")" 2> /dev/null; then
    fail "the tunnel runs already"
  fi
  nohup ssh -N -o BatchMode=yes -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 \
    -o ServerAliveCountMax=2 -R "127.0.0.1:$port:127.0.0.1:$port" "$host" > "$dir/tunnel.log" 2>&1 < /dev/null &
  echo $! > "$dir/tunnel.pid"
  sleep 3
  kill -0 "$(cat "$dir/tunnel.pid")" 2> /dev/null || fail "the tunnel exited: $(cat "$dir/tunnel.log")"
  # An sshd with GatewayPorts yes binds every address whatever the request says.
  bound=$(remote "ss -ltnH 'sport = :$port'" | awk '{ print $4 }')
  if [ -z "$bound" ] || grep -qv '^127\.0\.0\.1:' <<< "$bound"; then
    tunnel_down
    fail "the tunnel listens on ${bound:-nothing}, not on 127.0.0.1 only; stopped it"
  fi
  echo "tunnel: pid $(cat "$dir/tunnel.pid"), listening on $bound on $host"
}

tunnel_down() {
  $RUN || { say "kill the tunnel's ssh, by the pid in ~/$name/tunnel.pid"; return; }
  [ -e "$dir/tunnel.pid" ] && [ -e "$dir/port" ] || return 0
  local pid
  pid=$(cat "$dir/tunnel.pid")
  # Only while that pid still runs this tunnel, so a reused pid is spared.
  if tr '\0' ' ' < "/proc/$pid/cmdline" 2> /dev/null | grep -qF -- "-R 127.0.0.1:$(cat "$dir/port"):"; then
    kill "$pid"
  fi
  rm -f "$dir/tunnel.pid"
}

start_daemon() {
  $RUN || { say "priory daemon ~/$name/factory on the $1 machine"; return; }
  on "$1" '
    [ -e "$S/'"$marker"'" ] || { echo "no scratch folder at $S" >&2; exit 1; }
    if [ -e "$S/daemon.pid" ] && kill -0 "$(cat "$S/daemon.pid")" 2> /dev/null; then
      echo "the daemon runs already" >&2; exit 1
    fi
    nohup setsid priory daemon "$S/factory" >> "$S/daemon.log" 2>&1 < /dev/null &
    echo $! > "$S/daemon.pid"'
}

stop_daemon() {
  $RUN || { say "stop the $1 daemon, by the pid in ~/$name/daemon.pid"; return; }
  on "$1" '
    [ -e "$S/daemon.pid" ] || exit 0
    pid=$(cat "$S/daemon.pid")
    if tr "\0" " " < "/proc/$pid/cmdline" 2> /dev/null | grep -qF "priory daemon $S/factory"; then
      kill "$pid"
    fi
    rm -f "$S/daemon.pid"'
}

# Stops and removes only what carries this script's marker, and carries on past each failure.
down() {
  $RUN || {
    say "stop both daemons, the tunnel and the $container container"
    say "kill each machine's scratch tmux server and remove ~/$name where it holds $marker"
    return
  }
  stop_daemon local || echo "second-machine: could not stop the local daemon" >&2
  stop_daemon remote || echo "second-machine: could not stop the daemon on $host" >&2
  tunnel_down || echo "second-machine: could not stop the tunnel" >&2
  if [ -e "$dir/container" ]; then
    docker rm -f "$(cat "$dir/container")" > /dev/null || echo "second-machine: could not remove the container" >&2
  fi
  # The runs' tmux server lives under the scratch TMUX_TMPDIR, which only $S/env sets.
  local clean='[ -e "$S/'"$marker"'" ] || exit 0
    if [ -e "$S/env" ]; then tmux kill-server 2> /dev/null; fi
    rm -rf "$S"'
  on local "$clean" || echo "second-machine: could not remove ~/$name here" >&2
  on remote "$clean" || echo "second-machine: could not remove ~/$name on $host" >&2
}

run_priory() {
  $RUN || { say "priory ${*:2} in ~/$name/factory on the $1 machine"; return; }
  on "$1" 'cd "$S/factory" && exec priory "$@"' "${@:2}"
}

case $command in
  up) up ;;
  down) down ;;
  tunnel-up) tunnel_up ;;
  tunnel-down) tunnel_down ;;
  start) start_daemon "${1:?local or remote}" ;;
  stop) stop_daemon "${1:?local or remote}" ;;
  priory) run_priory "${1:?local or remote}" "${@:2}" ;;
  *) usage >&2; exit 2 ;;
esac
