"""How long a person's answer takes to be stored while 12 clients read and the CPU is busy.

Usage: python3 scripts/bench/answer-latency.py PRIORY DIR LOAD [ANSWERS]

PRIORY is the binary to measure, DIR a scratch folder it empties first, and LOAD one of steady (2
check runs a second), burst (1,000 at once every 15 s) or triage (a 3 s triage command on 1 message
in 20). ANSWERS defaults to 60. It serves on 127.0.0.1:7399, so run one at a time. It starts 8 busy
loops beside the daemon; to pin the CPU too, run it in a cgroup, as the pull request that added it did:

    systemd-run --user --scope -p CPUQuota=400% python3 scripts/bench/answer-latency.py ...

It prints p50, p95 and max in ms for: the answer POST to stored, a. a request's wait for a worker
(a POST the server refuses before the daemon), b. an answer sent on the daemon's socket, d. reading
asks and desk again after an answer, and each listing read by status. It reads each answer back
from the state, and says NOT STORED for one missing after its 200.
"""
import http.client, json, os, shutil, socket, sqlite3, subprocess, sys, tempfile, threading, time, glob, uuid
BIN, DIR, LOAD = sys.argv[1:4]
WANT = int(sys.argv[4]) if len(sys.argv) > 4 else 60
HERE = os.path.dirname(os.path.abspath(__file__)); ROOT = os.path.dirname(os.path.dirname(HERE))
SEED = f"{HERE}/answer-latency.sql"
PORT = 7399; CLIENTS = 12; BUSY = 8
shutil.rmtree(DIR, ignore_errors=True)
os.makedirs(f"{DIR}/home"); os.makedirs(f"{DIR}/tmux"); # A socket path holds at most 108 bytes, so the runtime folder stays short.
run = tempfile.mkdtemp(prefix="priory-bench-")
shutil.copytree(f"{ROOT}/examples/batches", f"{DIR}/factory")
toml = open(f"{DIR}/factory/factory.toml", "a")
toml.write('\n[serve]\nlisten = "127.0.0.1:7399"\ntoken_env = "PRIORY_TOKEN"\n')
if LOAD == "triage":
    import tomllib
    ev = open(f"{ROOT}/examples/decisions/triage.eval.toml").read()
    open(f"{DIR}/factory/triage.eval.toml", "w").write(ev)
    json.dump(tomllib.loads(ev)["cases"], open(f"{DIR}/factory/cases.json", "w"))
    shutil.copy(f"{HERE}/answer-latency-decide.py", f"{DIR}/factory/decide.py")
    toml.write('\n[decisions.triage]\nrun = "python3 decide.py"\nkey_env = "BENCH_KEY"\nevaluation = "triage.eval.toml"\nagreement = 0.9\nthreshold = 0.9\ntimeout = "5s"\ndaily_calls = 2000\n')
toml.close()
env = {k: v for k, v in os.environ.items() if not k.startswith(("PRIORY_", "HERDR_", "OTEL_")) and k not in
       ("TMUX", "DBUS_SESSION_BUS_ADDRESS", "HC_TERMINAL_ID", "CLAUDE_CODE_SESSION_ID", "LINEAR_API_KEY", "SLACK_BOT_TOKEN", "CLAUDE_CONFIG_DIR", "CODEX_HOME")}
env.update(PRIORY_DB=f"{DIR}/state.db", PRIORY_HOME=f"{DIR}/home", PRIORY_TOKEN="demo", HERDR_BUS_DB=f"{DIR}/bus.db", BENCH_KEY="x", PATH="/usr/bin:/bin",
           TMUX_TMPDIR=f"{DIR}/tmux", XDG_RUNTIME_DIR=run, PRIORY_SLICE="scratch-bench.slice")
subprocess.run([BIN, "tasks", "--json"], env=env, stdout=subprocess.DEVNULL, check=True)
db = sqlite3.connect(f"{DIR}/state.db"); db.executescript(open(SEED).read())
db.execute("INSERT INTO owns VALUES (1500, 'github:acme/shop#1500')"); db.commit(); db.close()
if LOAD == "triage": print(subprocess.run([BIN, "calibrate", "triage", f"{DIR}/factory"], env=env, capture_output=True, text=True).stdout.splitlines()[-1:])
log = open(f"{DIR}/daemon.log", "w")
daemon = subprocess.Popen([BIN, "daemon", f"{DIR}/factory"], env=env, stdout=log, stderr=log)
busy, stop, samples, lock = [], threading.Event(), {}, threading.Lock()
def note(name, s):
    with lock: samples.setdefault(name, []).append(s)
def call(method, path, body=None, headers={}):
    began = time.monotonic()
    try:
        c = http.client.HTTPConnection("127.0.0.1", PORT, timeout=60)
        c.request(method, path, body and json.dumps(body), {"Authorization": "Bearer demo", "Content-Type": "application/json", **headers})
        r = c.getresponse(); data = r.read(); c.close()
        return r.status, data, time.monotonic() - began
    except Exception as e: return 0, repr(e).encode(), time.monotonic() - began
def client():
    rev = -1
    def read(p):
        r = call("GET", "/api/" + p); note(f"read {p}" + ("" if r[0] == 200 else f" {r[0]}"), r[2])
    while not stop.is_set():
        status, body, took = call("GET", f"/api/events?since={rev}")
        if status != 200:
            note(f"events {status}", took); stop.wait(1 if status == 503 else 5); continue
        rev = json.loads(body)["revision"]
        ts = [threading.Thread(target=read, args=(p,)) for p in ("desk", "asks", "queue", "tasks")]
        [t.start() for t in ts]; [t.join() for t in ts]
def events():
    bus = sqlite3.connect(f"{DIR}/bus.db", timeout=20); n = 0
    while not stop.is_set():
        for _ in range(1000 if LOAD == "burst" else 1):
            n += 1; pr = 1441 + n % 59
            data = {"repo": "acme/shop", "pull_request": pr, "head_sha": "%040x" % n, "head_branch": f"eng-{pr}", "base_branch": "main", "name": "check", "conclusion": "success", "state": "open", "pad": "x" * 900}
            kind = "github.check_run.completed"
            if LOAD == "triage" and n % 20 == 0:
                kind = "github.pull_request.comment"
                data = {"repo": "acme/shop", "pull_request": 1500, "issue": 1500, "body": "SLOW thanks", "url": f"https://github.com/acme/shop/pull/1500#c{n}", "actor": "a", "association": "MEMBER"}
            at = time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime())
            bus.execute("INSERT OR IGNORE INTO outputs (name, declared, last_seen) VALUES ('factory.intake', ?, ?)", (at, at))
            bus.execute("INSERT INTO messages (output, id, type, subject, time, published, data) VALUES ('factory.intake',?,?,?,?,?,?)", (str(uuid.uuid4()), kind, f"acme/shop#{data['pull_request']}", at, at, json.dumps(data)))
        bus.commit(); stop.wait(15 if LOAD == "burst" else 0.5)
asks = list(range(400, 0, -1)); done = [0]
def press():
    state = sqlite3.connect(f"{DIR}/state.db", timeout=20)
    while done[0] < WANT and time.monotonic() < end:
        with lock: ask = asks.pop(); done[0] += 1
        r = call("POST", f"/api/asks/bench-{ask}/answer", {"option": "yes", "revision": 1}); note("answer POST" + ("" if r[0] == 200 else f" {r[0]}"), r[2])
        if r[0] == 200 and state.execute("SELECT answer FROM asks WHERE id=?", (ask,)).fetchone()[0] != "yes": note("NOT STORED after 200", 0)
        if r[0] == 200:
            a = call("GET", "/api/asks"); d = call("GET", "/api/desk"); note("d: re-read asks and desk", a[2] + d[2])
        time.sleep(0.3)
def beside(name, fn):
    while not stop.is_set():
        note(name, fn()); stop.wait(1)
def sock():
    with lock: ask = asks.pop()
    began = time.monotonic(); s = socket.socket(socket.AF_UNIX); s.settimeout(60); s.connect(SOCK)
    s.sendall((json.dumps({"ask": ask, "text": "yes", "revision": 1, "by": "cli"}) + "\n").encode()); s.makefile().readline(); s.close()
    return time.monotonic() - began
try:
    for _ in range(150):
        if call("GET", "/api/request")[0] == 200: break
        time.sleep(0.2)
    time.sleep(3)
    SOCK = max((p for p in glob.glob(f"{run}/**/*", recursive=True) + glob.glob(f"{DIR}/home/**/*", recursive=True) if os.path.exists(p) and os.stat(p).st_mode & 0o170000 == 0o140000), key=os.path.getmtime)
    busy = [subprocess.Popen(["sh", "-c", "while :; do :; done"]) for _ in range(BUSY)]
    for f in [client] * CLIENTS + [events]: threading.Thread(target=f, daemon=True).start()
    time.sleep(8); end = time.monotonic() + 240
    threading.Thread(target=beside, args=("b: socket answer", sock), daemon=True).start()
    threading.Thread(target=beside, args=("a: worker wait", lambda: call("POST", "/api/asks/1/answer", {"x": 1})[2]), daemon=True).start()
    ps = [threading.Thread(target=press) for _ in range(2)]; [p.start() for p in ps]; [p.join() for p in ps]
    with lock: out = {k: sorted(v) for k, v in samples.items()}
    for name, s in sorted(out.items()):
        q = lambda p: s[min(len(s) - 1, int(p * len(s)))] * 1000
        print(f"{name}: n={len(s)} p50={q(.5):.0f} p95={q(.95):.0f} max={s[-1]*1000:.0f}")
    print("cpu s of the daemon:", sum(map(int, open(f"/proc/{daemon.pid}/stat").read().split()[13:15])) / 100, "alive:", daemon.poll() is None, "events routed:", sqlite3.connect(f"{DIR}/state.db").execute("SELECT count(*) FROM inbox WHERE seq>30000").fetchone()[0])
finally:
    stop.set(); [b.kill() for b in busy]; daemon.terminate(); daemon.wait(20)
    shutil.rmtree(run, ignore_errors=True)
